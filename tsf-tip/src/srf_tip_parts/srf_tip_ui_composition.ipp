void CSrfTip::EnsureTrayHelperRunningAsync() {
  static std::atomic<bool> inFlight{false};
  if (inFlight.exchange(true, std::memory_order_acq_rel)) return;
  SrfTip_BackgroundWorkerAddRef();
  try {
    std::thread([] {
      try {
        EnsureTrayHelperRunning();
      } catch (...) {
      }
      inFlight.store(false, std::memory_order_release);
      SrfTip_BackgroundWorkerRelease();
    }).detach();
  } catch (...) {
    inFlight.store(false, std::memory_order_release);
    SrfTip_BackgroundWorkerRelease();
  }
}

HRESULT CSrfTip::_UnadviseSinks() {
  CancelDeferredCandidateRefresh();
  CancelScheduledCandidateUiRedraw();
  StopCandidateLookupWorker();
  if (m_candidateUi) m_candidateUi->End();
  m_notificationWindow.Hide();
  ReleaseCompositionState();

  if (m_pFocusContext) {
    m_pFocusContext->Release();
    m_pFocusContext = nullptr;
  }

  UnregisterPreservedKeys();

  if (m_pKeystrokeMgr && m_tid != 0) {
    (void)m_pKeystrokeMgr->UnadviseKeyEventSink(m_tid);
  }

  if (m_pSource && m_dwThreadMgrSinkCookie != TF_INVALID_COOKIE) {
    (void)m_pSource->UnadviseSink(m_dwThreadMgrSinkCookie);
    m_dwThreadMgrSinkCookie = TF_INVALID_COOKIE;
  }

  if (m_pThreadMgrSink) {
    m_pThreadMgrSink->Release();
    m_pThreadMgrSink = nullptr;
  }
  if (m_pKeySink) {
    m_pKeySink->Release();
    m_pKeySink = nullptr;
  }
  if (m_pCompSink) {
    m_pCompSink->Release();
    m_pCompSink = nullptr;
  }

  if (m_pCompartmentMgr) {
    m_pCompartmentMgr->Release();
    m_pCompartmentMgr = nullptr;
  }
  if (m_pSource) {
    m_pSource->Release();
    m_pSource = nullptr;
  }
  if (m_pKeystrokeMgr) {
    m_pKeystrokeMgr->Release();
    m_pKeystrokeMgr = nullptr;
  }
  if (m_pThreadMgr) {
    m_pThreadMgr->Release();
    m_pThreadMgr = nullptr;
  }

  m_tid = 0;
  m_displayAttrAtom = TF_INVALID_GUIDATOM;
  m_dwThreadMgrSinkCookie = TF_INVALID_COOKIE;
  m_lastActivationTick = 0;
  m_ignoreImeToggleUntilModifiersReleased = false;
  m_focusCancelSequence = 0;
  m_activeAppName.clear();
  m_cachedFocusedHwnd = nullptr;
  m_cachedFocusedProcessId = 0;
  m_cachedFocusedProcessName.clear();
  return S_OK;
}

constexpr UINT kCandidateAnchorQualityUnsafe = 0;
constexpr UINT kCandidateAnchorQualityScreenSafe = 1;
constexpr UINT kCandidateAnchorQualityMouse = 1;
constexpr UINT kCandidateAnchorQualityWindow = 2;
constexpr UINT kCandidateAnchorQualityCaret = 3;
constexpr UINT kCandidateAnchorQualityTextExt = 4;

bool CandidateAnchorCanStick(UINT quality) {
  return quality >= kCandidateAnchorQualityWindow;
}

int CandidateAnchorStickThresholdPx(const RECT& rect) {
  return MulDiv(24, static_cast<int>(DpiForScreenRect(&rect)), 96);
}

bool CandidateAnchorMovedFar(const RECT& oldRect, const RECT& newRect) {
  const int threshold = CandidateAnchorStickThresholdPx(oldRect);
  const int oldX = (oldRect.left + oldRect.right) / 2;
  const int newX = (newRect.left + newRect.right) / 2;
  const int oldY = oldRect.bottom;
  const int newY = newRect.bottom;
  return std::abs(newX - oldX) > threshold || std::abs(newY - oldY) > threshold;
}

bool CandidateAnchorMovedBeyondHysteresis(const RECT& oldRect, const RECT& newRect) {
  const int threshold = MulDiv(8, static_cast<int>(DpiForScreenRect(&oldRect)), 96);
  const int oldX = (oldRect.left + oldRect.right) / 2;
  const int newX = (newRect.left + newRect.right) / 2;
  const int oldY = oldRect.bottom;
  const int newY = newRect.bottom;
  return std::abs(newX - oldX) > threshold || std::abs(newY - oldY) > threshold;
}

bool CandidateAnchorCrossedMonitor(const RECT& oldRect, const RECT& newRect) {
  HMONITOR oldMonitor = MonitorFromRect(&oldRect, MONITOR_DEFAULTTONEAREST);
  HMONITOR newMonitor = MonitorFromRect(&newRect, MONITOR_DEFAULTTONEAREST);
  return oldMonitor && newMonitor && oldMonitor != newMonitor;
}

bool ShouldReplaceStickyCandidateAnchor(const RECT& stickyRect, UINT stickyQuality,
                                        const RECT& newRect, UINT newQuality) {
  if (!IsUsableRect(stickyRect)) return true;
  if (CandidateAnchorCrossedMonitor(stickyRect, newRect)) return true;
  if (newQuality > stickyQuality) return true;
  return CandidateAnchorMovedFar(stickyRect, newRect);
}

HWND CSrfTip::CandidateOverlayTargetWindow() const {
  ITfContext* context = m_pCompositionContext ? m_pCompositionContext : m_pFocusContext;
  HWND contextHwnd = context ? RootWindowForPlacement(ResolveContextWindow(context)) : nullptr;
  if (contextHwnd && (!IsWindow(contextHwnd) || !IsWindowVisible(contextHwnd))) {
    contextHwnd = nullptr;
  }

  HWND foreground = RootWindowForPlacement(GetForegroundWindow());
  if (foreground && (!IsWindow(foreground) || !IsWindowVisible(foreground))) {
    foreground = nullptr;
  }
  if (!foreground) return contextHwnd;
  if (!contextHwnd) return foreground;

  bool appGameProfile = false;
  if (const SrfAppOptions* options = FindAppOptions(m_config, m_activeAppName)) {
    appGameProfile = options->hasGameProfile && options->gameCompactProfile;
  }
  if (appGameProfile || m_fullscreenCompatActive || m_gameCompatActive ||
      m_configuredGameCompatActive || m_builtinGameCompatActive ||
      m_manualGameCompatActive) {
    return foreground;
  }

  DWORD contextPid = 0;
  DWORD foregroundPid = 0;
  (void)GetWindowThreadProcessId(contextHwnd, &contextPid);
  (void)GetWindowThreadProcessId(foreground, &foregroundPid);
  return contextPid != 0 && contextPid == foregroundPid ? foreground : contextHwnd;
}

bool CSrfTip::CandidateGameOverlayActive() const {
  if (EffectiveCompatibilityPolicy() != SrfFullscreenPolicy::ShowUi) return false;
  bool appGameProfile = false;
  if (const SrfAppOptions* options = FindAppOptions(m_config, m_activeAppName)) {
    appGameProfile = options->hasGameProfile && options->gameCompactProfile;
  }
  return appGameProfile || m_fullscreenCompatActive || m_gameCompatActive ||
         m_configuredGameCompatActive || m_builtinGameCompatActive || m_manualGameCompatActive;
}

bool CSrfTip::FullscreenCandidateOverlayActive() const {
  if (!CandidateGameOverlayActive()) return false;
  HWND target = CandidateOverlayTargetWindow();
  return target && IsFullscreenForegroundWindow(target);
}

void CSrfTip::RefreshCandidateWindowEnvironment() {
  if (!m_candidateUi || !m_status.composing || m_reading.empty() ||
      m_context.candidates.Empty()) {
    return;
  }

  RefreshCompatibilityStateThrottled(true);
  HWND target = CandidateOverlayTargetWindow();
  const bool gameOverlay = CandidateGameOverlayActive();
  const bool fullscreenOverlay = FullscreenCandidateOverlayActive();
  const SrfAppOptions* overlayOptions = FindAppOptions(m_config, m_activeAppName);
  const SrfOverlayAnchor overlayAnchor = EffectiveOverlayAnchor(overlayOptions);

  RECT rect = {};
  bool foundRect = false;
  const wchar_t* source = L"environment-existing";
  UINT quality = m_lastCandidateAnchorQuality;
  if (gameOverlay && overlayAnchor != SrfOverlayAnchor::Caret) {
    foundRect = ResolveCandidateGameOverlayAnchor(
        target, fullscreenOverlay, overlayOptions, &rect);
    if (foundRect) {
      source = fullscreenOverlay ? L"fullscreen-overlay-environment"
                                 : L"game-overlay-environment";
      quality = kCandidateAnchorQualityWindow;
    }
  } else if (m_hasLastCandidateRect && IsUsableRect(m_lastCandidateRect)) {
    rect = m_lastCandidateRect;
    foundRect = true;
  }
  if (!foundRect) {
    foundRect = TryGetScreenSafeRect(target, &rect);
    if (foundRect) {
      source = L"screen-safe-environment";
      quality = kCandidateAnchorQualityScreenSafe;
    }
  }

  if (foundRect) {
    m_lastCandidateRect = rect;
    m_hasLastCandidateRect = true;
    m_lastCandidateAnchorSource = source;
    m_lastCandidateAnchorQuality = quality;
  } else {
    m_hasLastCandidateRect = false;
  }
  InvalidateCandidatePageLayoutCache();

  std::wstring line = L"source=";
  line += source;
  line += L", overlay=";
  line += fullscreenOverlay ? L"1" : L"0";
  line += L", target=";
  line += FormatPointerForLog(target);
  line += L", rect=";
  line += foundRect ? FormatRectForLog(rect) : L"(none)";
  SrfTsfDiagnosticLog(L"candidate-window.environment-refresh", line.c_str());
  RedrawCandidateUiImmediate();
  if (gameOverlay && !fullscreenOverlay) {
    ScheduleCandidateUiRedraw(static_cast<DWORD>(kCompatibilityReleaseDebounceMs + 20));
  }
}

void CSrfTip::UpdateCandidateWindow(TfEditCookie ec) {
  // Any successful edit session or normal key path now owns a freshly sampled
  // TSF coordinate space, so the external helper may be used again.
  if (m_candidateUi) m_candidateUi->OnCandidateAnchorRefreshed();

  const bool hadPreviousRect = m_hasLastCandidateRect;
  const RECT previousRect = m_lastCandidateRect;

  if (m_reading.empty()) {
    if (m_hasLastCandidateRect) {
      m_hasLastCandidateRect = false;
      InvalidateCandidatePageLayoutCache();
    }
    SrfTsfDiagnosticLog(L"candidate-anchor.skip", L"reading is empty");
    RedrawCandidateUi();
    return;
  }

  // 单字续组词刚建立的新 composition 在部分宿主中会短暂返回错误的
  // TextExt 坐标。首批候选到达时沿用提交前的锚点；下一次正常按键会重新采样。
  if (!m_preserveCandidateAnchorReading.empty()) {
    const bool preserve = m_preserveCandidateAnchorReading == m_reading && hadPreviousRect;
    m_preserveCandidateAnchorReading.clear();
    if (preserve) {
      SrfTsfPerfLog(L"candidate-anchor.preserve", L"reason=single-char-continuation");
      RedrawCandidateUi();
      return;
    }
  }

  ITfContext* context = m_pCompositionContext ? m_pCompositionContext : m_pFocusContext;
  HWND contextHwnd = context ? ResolveContextWindow(context) : nullptr;
  const SrfFocusPolicy focusPolicy = EffectiveFocusPolicy();
  const bool gameOverlay = CandidateGameOverlayActive();
  const bool fullscreenOverlay = FullscreenCandidateOverlayActive();
  const SrfAppOptions* overlayOptions = FindAppOptions(m_config, m_activeAppName);
  const SrfOverlayAnchor overlayAnchor = EffectiveOverlayAnchor(overlayOptions);
  const bool fixedGameOverlay = gameOverlay && overlayAnchor != SrfOverlayAnchor::Caret;
  const bool windowFocusPolicy = focusPolicy == SrfFocusPolicy::Window;
  const bool allowTextExt = !fixedGameOverlay && !windowFocusPolicy;
  const bool allowGuiCaret =
      !fixedGameOverlay && m_uiStyle.enhancedPosition && !windowFocusPolicy;
  const bool allowMouseFallback =
      !fixedGameOverlay && m_uiStyle.enhancedPosition && focusPolicy == SrfFocusPolicy::Normal;
  const bool allowForegroundFallback =
      !fixedGameOverlay && focusPolicy == SrfFocusPolicy::Normal;
  const bool allowScreenSafeFallback = focusPolicy == SrfFocusPolicy::Normal || contextHwnd != nullptr;
  RECT rect = {};
  bool foundRect = false;
  const wchar_t* anchorSource = L"none";
  UINT anchorQuality = kCandidateAnchorQualityUnsafe;
  bool overlayOffsetApplied = false;

  if (fixedGameOverlay) {
    HWND hwnd = CandidateOverlayTargetWindow();
    if (!hwnd) hwnd = contextHwnd;
    foundRect = ResolveCandidateGameOverlayAnchor(
        hwnd, fullscreenOverlay, overlayOptions, &rect);
    if (foundRect) {
      anchorSource = fullscreenOverlay ? L"fullscreen-overlay" : L"game-overlay";
      anchorQuality = kCandidateAnchorQualityWindow;
      overlayOffsetApplied = true;
    }
  }

  if (context && m_pComposition && allowTextExt) {
    ITfRange* range = nullptr;
    if (SUCCEEDED(m_pComposition->GetRange(&range)) && range) {
      foundRect = TryGetTextExtRect(ec, context, range, &rect);
      if (foundRect) {
        anchorSource = L"tsf-text-ext";
        anchorQuality = kCandidateAnchorQualityTextExt;
      }
      range->Release();
    }
  }

  if (!foundRect && allowGuiCaret) {
    foundRect = TryGetGuiCaretRect(&rect);
    if (foundRect) {
      anchorSource = L"gui-caret";
      anchorQuality = kCandidateAnchorQualityCaret;
    }
  }
  if (!foundRect && context) {
    HWND hwnd = contextHwnd;
    if (hwnd) {
      foundRect = TryGetWindowBottomLeftRect(hwnd, &rect);
      if (foundRect) {
        anchorSource = L"context-window-bottom-left";
        anchorQuality = kCandidateAnchorQualityWindow;
      }
    }
  }
  if (!foundRect && context) {
    HWND hwnd = contextHwnd;
    if (hwnd) {
      RECT client = {};
      if (GetClientRect(hwnd, &client)) {
        MapWindowPoints(hwnd, nullptr, reinterpret_cast<POINT*>(&client), 2);
        rect = client;
        rect.left = rect.right;
        rect.right += 1;
        rect.bottom = std::max(rect.top + 20, rect.bottom);
        foundRect = IsUsableRect(rect);
        if (foundRect) {
          anchorSource = L"context-window";
          anchorQuality = kCandidateAnchorQualityWindow;
        }
      }
    }
  }
  if (!foundRect && allowMouseFallback) {
    foundRect = TryGetMouseRect(contextHwnd, &rect);
    if (foundRect) {
      anchorSource = L"mouse";
      anchorQuality = kCandidateAnchorQualityMouse;
    }
  }
  if (!foundRect && allowForegroundFallback) {
    HWND hwnd = GetForegroundWindow();
    foundRect = TryGetWindowBottomLeftRect(hwnd, &rect);
    if (foundRect) {
      anchorSource = L"foreground-window-bottom-left";
      anchorQuality = kCandidateAnchorQualityWindow;
    }
  }
  if (!foundRect && allowScreenSafeFallback) {
    HWND hwnd = contextHwnd ? contextHwnd : GetForegroundWindow();
    foundRect = TryGetScreenSafeRect(hwnd, &rect);
    if (foundRect) {
      anchorSource = L"screen-safe";
      anchorQuality = kCandidateAnchorQualityScreenSafe;
    }
  }

  if (foundRect && gameOverlay && !overlayOffsetApplied) {
    ApplyCandidateGameOverlayOffset(overlayOptions, &rect);
    overlayOffsetApplied = true;
  }

  if (!fixedGameOverlay && m_hasStickyCandidateRect && IsUsableRect(m_stickyCandidateRect)) {
    if (!foundRect || !CandidateAnchorCanStick(anchorQuality)) {
      rect = m_stickyCandidateRect;
      foundRect = true;
      anchorSource = L"sticky";
      anchorQuality = m_stickyCandidateAnchorQuality;
    } else if (ShouldReplaceStickyCandidateAnchor(m_stickyCandidateRect,
                                                  m_stickyCandidateAnchorQuality,
                                                  rect, anchorQuality)) {
      m_stickyCandidateRect = rect;
      m_stickyCandidateAnchorQuality = anchorQuality;
    } else {
      rect = m_stickyCandidateRect;
      anchorSource = L"sticky";
      anchorQuality = m_stickyCandidateAnchorQuality;
    }
  } else if (foundRect && CandidateAnchorCanStick(anchorQuality)) {
    m_stickyCandidateRect = rect;
    m_hasStickyCandidateRect = true;
    m_stickyCandidateAnchorQuality = anchorQuality;
  }

  if (foundRect) {
    std::wstring anchorSourceText = anchorSource;
    const ULONGLONG now = GetTickCount64();
    const bool sourceChanged =
        !m_lastCandidateAnchorSource.empty() &&
        m_lastCandidateAnchorSource != anchorSourceText;
    const bool rapidSourceSwitch =
        m_lastCandidateAnchorSourceSwitchTick != 0 &&
        now >= m_lastCandidateAnchorSourceSwitchTick &&
        now - m_lastCandidateAnchorSourceSwitchTick <= 48;
    if (sourceChanged && hadPreviousRect && rapidSourceSwitch &&
        anchorQuality <= m_lastCandidateAnchorQuality &&
        !CandidateAnchorCrossedMonitor(previousRect, rect) &&
        !CandidateAnchorMovedBeyondHysteresis(previousRect, rect)) {
      std::wstring line = L"from=";
      line += m_lastCandidateAnchorSource;
      line += L", to=";
      line += anchorSourceText;
      line += L", rect=";
      line += FormatRectForLog(rect);
      line += L", kept=";
      line += FormatRectForLog(previousRect);
      line += L", reading=";
      line += ShortenForLog(m_reading, 24);
      SrfTsfPerfLog(L"candidate-anchor.hysteresis", line.c_str());
      rect = previousRect;
      anchorSourceText = m_lastCandidateAnchorSource;
      anchorQuality = m_lastCandidateAnchorQuality;
    } else if (sourceChanged) {
      ++m_candidateAnchorSourceSwitchCount;
      m_lastCandidateAnchorSourceSwitchTick = now;
      std::wstring line = L"from=";
      line += m_lastCandidateAnchorSource;
      line += L", to=";
      line += anchorSourceText;
      line += L", count=";
      line += std::to_wstring(m_candidateAnchorSourceSwitchCount);
      line += L", reading=";
      line += ShortenForLog(m_reading, 24);
      SrfTsfPerfLog(L"candidate-anchor.switch", line.c_str());
    }
    m_lastCandidateAnchorSource = anchorSourceText;
    m_lastCandidateAnchorQuality = anchorQuality;

    m_lastCandidateRect = rect;
    m_hasLastCandidateRect = true;
    if (!hadPreviousRect || previousRect.left != rect.left || previousRect.top != rect.top ||
        previousRect.right != rect.right || previousRect.bottom != rect.bottom) {
      InvalidateCandidatePageLayoutCache();
    }
    DebugLogCandidateAnchorSource(anchorSourceText.c_str(), rect);
    std::wstring line = L"source=";
    line += anchorSourceText;
    line += L", rect=";
    line += FormatRectForLog(rect);
    line += L", quality=";
    line += std::to_wstring(anchorQuality);
    line += L", source_switches=";
    line += std::to_wstring(m_candidateAnchorSourceSwitchCount);
    line += L", reading=";
    line += ShortenForLog(m_reading, 24);
    line += L", focusPolicy=";
    line += EffectiveFocusPolicyName();
    line += L", focus=";
    line += FormatFocusSnapshotForLog(CaptureFocusSnapshot(context));
    SrfTsfPerfLog(L"candidate-anchor.ok", line.c_str());
  } else {
    std::wstring line = L"reading=" + ShortenForLog(m_reading, 24);
    line += L", context=";
    line += context ? L"yes" : L"no";
    line += L", enhanced=";
    line += m_uiStyle.enhancedPosition ? L"1" : L"0";
    line += L", focusPolicy=";
    line += EffectiveFocusPolicyName();
    line += L", focus=";
    line += FormatFocusSnapshotForLog(CaptureFocusSnapshot(context));
    SrfTsfDiagnosticLog(L"candidate-anchor.missing", line.c_str());
    if (m_hasLastCandidateRect) {
      m_hasLastCandidateRect = false;
      InvalidateCandidatePageLayoutCache();
    }
  }

  RedrawCandidateUi();
}

void CSrfTip::RedrawCandidateUi() {
  RedrawCandidateUiImmediate();
}

void CSrfTip::RedrawCandidateUiImmediate() {
  CancelScheduledCandidateUiRedraw();
  RedrawCandidateUiNow();
}

void CSrfTip::RedrawCandidateUiNow() {
  if (!m_candidateUi) {
    SrfTsfDiagnosticLog(L"candidate-ui.skip", L"candidate ui element is null");
    return;
  }
  RefreshCompatibilityStateThrottled();
  if (ShouldHideUiForCompatibility()) {
    m_candidateUiTransientMissingSince = 0;
    std::wstring line = L"policy=";
    line += EffectiveCompatibilityPolicyName();
    line += L", app=";
    line += m_activeAppName.empty() ? L"(unknown)" : m_activeAppName;
    line += L", engine=";
    line += EngineStateName(SrfTip_GetEngineState());
    line += L", uiLess=";
    line += m_uiLessMode ? L"1" : L"0";
    line += L", count=";
    line += std::to_wstring(m_candidates.size());
    line += L", hasAnchor=";
    line += m_hasLastCandidateRect ? L"1" : L"0";
    line += L", composing=";
    line += m_status.composing ? L"1" : L"0";
    SrfTsfDiagnosticLog(L"candidate-ui.hidden", line.c_str());
    m_candidateUi->End();
    return;
  }
  if (m_status.composing && !m_candidates.empty() && m_candidatesReading != m_reading) {
    SetCandidateViewState(SrfCandidateViewState::Stale, L"redraw-stale");
  }
  if (!m_status.composing || m_context.candidates.Empty() || !m_hasLastCandidateRect) {
    const bool transientWhileComposing = m_status.composing && !m_reading.empty();
    if (transientWhileComposing) {
      const ULONGLONG now = GetTickCount64();
      if (m_candidateUiTransientMissingSince == 0) {
        m_candidateUiTransientMissingSince = now;
      }
      const ULONGLONG elapsed = now - m_candidateUiTransientMissingSince;
      if (elapsed < kCandidateUiTransientHideGraceMs) {
        std::wstring transientLine = L"composing=1, grace_ms_left=";
        transientLine += std::to_wstring(kCandidateUiTransientHideGraceMs - elapsed);
        transientLine += L", contextEmpty=";
        transientLine += m_context.candidates.Empty() ? L"1" : L"0";
        transientLine += L", hasAnchor=";
        transientLine += m_hasLastCandidateRect ? L"1" : L"0";
        transientLine += L", reading=";
        transientLine += ShortenForLog(m_reading, 24);
        SrfTsfPerfLog(L"candidate-ui.keepalive", transientLine.c_str());
        ScheduleCandidateUiRedraw(static_cast<DWORD>(kCandidateUiTransientHideGraceMs - elapsed));
        return;
      }
    } else {
      m_candidateUiTransientMissingSince = 0;
    }
    std::wstring line = L"composing=";
    line += m_status.composing ? L"1" : L"0";
    line += L", contextEmpty=";
    line += m_context.candidates.Empty() ? L"1" : L"0";
    line += L", hasAnchor=";
    line += m_hasLastCandidateRect ? L"1" : L"0";
    line += L", reading=";
    line += ShortenForLog(m_reading, 24);
    line += L", app=";
    line += m_activeAppName.empty() ? L"(unknown)" : m_activeAppName;
    line += L", engine=";
    line += EngineStateName(SrfTip_GetEngineState());
    line += L", uiLess=";
    line += m_uiLessMode ? L"1" : L"0";
    line += L", policy=";
    line += EffectiveCompatibilityPolicyName();
    line += L", count=";
    line += std::to_wstring(m_candidates.size());
    SrfTsfPerfLog(L"candidate-ui.end", line.c_str());
    m_candidateUi->End();
    m_candidateUiTransientMissingSince = 0;
    return;
  }
  m_candidateUiTransientMissingSince = 0;
  std::wstring line = L"count=";
  line += std::to_wstring(m_candidates.size());
  line += L", page=";
  line += std::to_wstring(m_candPage + 1);
  line += L", selected=";
  line += std::to_wstring(m_candSel);
  line += L", anchor=";
  line += FormatRectForLog(m_lastCandidateRect);
  line += L", app=";
  line += m_activeAppName.empty() ? L"(unknown)" : m_activeAppName;
  line += L", engine=";
  line += EngineStateName(SrfTip_GetEngineState());
  line += L", uiLess=";
  line += m_uiLessMode ? L"1" : L"0";
  line += L", policy=";
  line += EffectiveCompatibilityPolicyName();
  line += L", composing=";
  line += m_status.composing ? L"1" : L"0";
  line += L", viewState=";
  line += CandidateViewStateName();
  const ULONGLONG now = GetTickCount64();
  const ULONGLONG stateAgeMs =
      m_candidateViewStateSinceTick == 0 || now < m_candidateViewStateSinceTick
          ? 0
          : now - m_candidateViewStateSinceTick;
  line += L", state_age_ms=";
  line += std::to_wstring(stateAgeMs);
  SrfTsfPerfLog(L"candidate-ui.show", line.c_str());
  const ULONGLONG redrawStart = now;
  const ULONGLONG prepStart = redrawStart;
  m_candidateUi->PrepareWindowResources();
  DebugLogPerfMs(L"CandidateWindow/prepare-resources", prepStart);
  const ULONGLONG beginStart = GetTickCount64();
  const HRESULT beginHr = m_candidateUi->BeginOrUpdate();
  if (FAILED(beginHr) && m_fullscreenCompatActive &&
      EffectiveCompatibilityPolicy() == SrfFullscreenPolicy::ShowUi) {
    RecordCompatibilityUiFallback(L"CandidateUIBeginOrUpdate", beginHr);
  }
  DebugLogPerfMs(L"CandidateWindow/begin-or-update", beginStart);
  DebugLogPerfMs(L"CandidateWindow/total", redrawStart);
}

HRESULT CSrfTip::SyncCompositionText(TfEditCookie ec, ITfContext* pic, bool refreshCandidates) {
  if (!pic) return E_INVALIDARG;
  ClampReadingCursor();

  std::wstring syncLine = L"reading=" + ShortenForLog(m_reading);
  syncLine += L", refreshCandidates=";
  syncLine += refreshCandidates ? L"1" : L"0";
  syncLine += L", cursor=";
  syncLine += std::to_wstring(m_readingCursor);
  SrfTsfPerfLog(L"sync-composition.begin", syncLine.c_str());

  if (m_reading.empty()) {
    CancelCompositionEdit(ec);
    return S_OK;
  }

  bool updateCandidateWindowNow = true;
  if (refreshCandidates) {
    // 用户继续输入/回删导致 reading 变化时，应回到第一页候选。
    // 否则“翻页后再输入第二个音节”会停留在旧页码，造成候选视图不符合预期。
    m_candSel = 0;
    m_candPage = 0;
    updateCandidateWindowNow = RefreshCandidatesAsync();
  }

  HRESULT hr = E_FAIL;
  for (int attempt = 0; attempt < 2; ++attempt) {
    hr = EnsureComposition(ec, pic);
    if (FAILED(hr)) {
      ReleaseCompositionObjects();
      continue;
    }

    hr = SetCompositionDisplay(ec, m_pComposition, BuildCompositionDisplay());
    if (SUCCEEDED(hr)) break;

    ReleaseCompositionObjects();
  }
  if (FAILED(hr)) return hr;

  (void)ApplyCompositionDisplayAttribute(ec, pic);
  (void)SetCompositionSelection(ec, pic);
  if (updateCandidateWindowNow) {
    UpdateCandidateWindow(ec);
  } else {
    if (m_candidateUi) m_candidateUi->UpdatePresentationState();
    std::wstring line = L"reading=";
    line += ShortenForLog(m_reading, 24);
    line += L", reason=await_async_candidate";
    SrfTsfPerfLog(L"candidate-ui.defer", line.c_str());
  }
  SrfTsfPerfLog(L"sync-composition.end", L"composition synced");
  return S_OK;
}

HRESULT CSrfTip::EnsureComposition(TfEditCookie ec, ITfContext* pic) {
  if (!pic) return E_INVALIDARG;

  if (m_pComposition) return S_OK;
  if (m_pCompositionContext) {
    m_pCompositionContext->Release();
    m_pCompositionContext = nullptr;
  }

  ITfContextComposition* contextComposition = nullptr;
  HRESULT hr =
      pic->QueryInterface(IID_ITfContextComposition, reinterpret_cast<void**>(&contextComposition));
  if (FAILED(hr) || !contextComposition) {
    if (SrfTsfDebugTraceEnabled()) {
      wchar_t buf[120] = {};
      swprintf_s(buf, L"EnsureComposition: QI ITfContextComposition failed hr=0x%08lX",
                 static_cast<unsigned long>(hr));
      SrfTsfDebugLog(buf);
    }
    RecordCompatibilityFallback(L"ITfContextComposition", FAILED(hr) ? hr : E_FAIL);
    return FAILED(hr) ? hr : E_FAIL;
  }

  ITfRange* insertion = nullptr;
  hr = GetInsertionRange(ec, pic, &insertion);
  if (SUCCEEDED(hr) && insertion) {
    hr = contextComposition->StartComposition(ec, insertion, static_cast<ITfCompositionSink*>(m_pCompSink),
                                              &m_pComposition);
    insertion->Release();
  }
  contextComposition->Release();
  if (FAILED(hr) || !m_pComposition) {
    if (SrfTsfDebugTraceEnabled()) {
      wchar_t buf[120] = {};
      swprintf_s(buf, L"EnsureComposition: StartComposition failed hr=0x%08lX comp=%p",
                 static_cast<unsigned long>(hr), static_cast<void*>(m_pComposition));
      SrfTsfDebugLog(buf);
    }
    if (m_pComposition) {
      m_pComposition->Release();
      m_pComposition = nullptr;
    }
    RecordCompatibilityFallback(L"StartComposition", FAILED(hr) ? hr : E_FAIL);
    return FAILED(hr) ? hr : E_FAIL;
  }

  m_pCompositionContext = pic;
  m_pCompositionContext->AddRef();
  ++m_compositionGeneration;
  return S_OK;
}

HRESULT CSrfTip::CommitDirectText(TfEditCookie ec, ITfContext* pic, const std::wstring& text) {
  return CommitDirectTextWithCursor(ec, pic, text, -1);
}

HRESULT CSrfTip::CommitDirectTextWithCursor(TfEditCookie ec, ITfContext* pic,
                                            const std::wstring& text, LONG cursorOffset) {
  if (!pic) return E_INVALIDARG;
  if (text.empty()) return S_OK;

  const LONG textLength = static_cast<LONG>(text.size());
  if (cursorOffset < 0 || cursorOffset > textLength) cursorOffset = textLength;
  const bool cursorAtEnd = cursorOffset == textLength;

  // Clean up any stale composition first.
  if (m_pComposition) {
    CancelCompositionEdit(ec);
    ReleaseCompositionObjects();
  }

  const SrfCommitTransport commitTransport = EffectiveCommitTransport();
  if (cursorAtEnd && commitTransport != SrfCommitTransport::Tsf) {
    HRESULT transportHr = E_FAIL;
    if (commitTransport == SrfCommitTransport::ClipboardPaste) {
      if (ShouldSuppressClipboardForPrivacy()) {
        transportHr = HRESULT_FROM_WIN32(ERROR_ACCESS_DENIED);
      } else {
        transportHr = PasteUnicodeTextViaClipboard(text);
      }
    } else if (commitTransport == SrfCommitTransport::UnicodeSendInput) {
      transportHr = SendUnicodeTextInput(text);
    }

    std::wstring line = L"transport=";
    line += EffectiveCommitTransportName();
    line += L", status=";
    line += SUCCEEDED(transportHr) ? L"ok" : L"fallback_tsf";
    line += L", len=";
    line += std::to_wstring(text.size());
    if (FAILED(transportHr)) {
      wchar_t hrBuf[16] = {};
      swprintf_s(hrBuf, L"%08lX", static_cast<unsigned long>(transportHr));
      line += L", hr=0x";
      line += hrBuf;
    }
    SrfTsfDiagnosticLog(L"commit-transport", line.c_str());

    if (SUCCEEDED(transportHr)) return S_OK;
  }

  auto insertAtSelectionOnly = [&]() -> HRESULT {
    ITfInsertAtSelection* insertAtSelection = nullptr;
    HRESULT hr = pic->QueryInterface(IID_ITfInsertAtSelection, reinterpret_cast<void**>(&insertAtSelection));
    if (FAILED(hr) || !insertAtSelection) {
      if (SrfTsfDebugTraceEnabled()) {
        wchar_t buf[120] = {};
        swprintf_s(buf, L"CommitDirectText: QI ITfInsertAtSelection failed hr=0x%08lX",
                   static_cast<unsigned long>(hr));
        SrfTsfDebugLog(buf);
      }
      return FAILED(hr) ? hr : E_FAIL;
    }

    ITfRange* insertedRange = nullptr;
    hr = insertAtSelection->InsertTextAtSelection(ec, 0, text.c_str(), static_cast<LONG>(text.size()),
                                                  &insertedRange);
    insertAtSelection->Release();
    if (SUCCEEDED(hr) && insertedRange) {
      (void)CollapseSelectionToRangeOffset(ec, pic, insertedRange, cursorOffset);
      insertedRange->Release();
    }
    if (FAILED(hr) && SrfTsfDebugTraceEnabled()) {
      wchar_t buf[120] = {};
      swprintf_s(buf, L"CommitDirectText: InsertTextAtSelection failed hr=0x%08lX len=%zu",
                 static_cast<unsigned long>(hr), text.size());
      SrfTsfDebugLog(buf);
    }
    return hr;
  };

  // Use a composition to insert text. This is the standard TSF approach and
  // works reliably with all applications, including CUAS mode where
  // InsertTextAtSelection alone may not produce visible output.
  //
  // 但对小键盘数字/符号及中文标点这类“直出”输入，部分宿主会把 composition 过程渲染成带下划线
  // 的预编辑，甚至表现为需要再次按回车才完成上屏。为避免该体验，优先走 InsertTextAtSelection。
  if (text.size() == 1) {
    const wchar_t ch = text[0];
    if (IsDirectInsertPreferredChar(ch)) {
      const HRESULT hr = insertAtSelectionOnly();
      if (SUCCEEDED(hr)) return hr;
      // 若直插失败，再回退到 composition 路径。
    }
  } else if (!cursorAtEnd) {
    const HRESULT hr = insertAtSelectionOnly();
    if (SUCCEEDED(hr)) return hr;
  }

  HRESULT hr = EnsureComposition(ec, pic);
  if (SUCCEEDED(hr) && m_pComposition) {
    ITfRange* range = nullptr;
    hr = m_pComposition->GetRange(&range);
    if (SUCCEEDED(hr) && range) {
      hr = range->SetText(ec, 0, text.c_str(), static_cast<LONG>(text.size()));
      if (SUCCEEDED(hr)) {
        (void)CollapseSelectionToRangeOffset(ec, pic, range, cursorOffset);
      }
      if (SUCCEEDED(hr)) {
        const HRESULT endHr = m_pComposition->EndComposition(ec);
        if (SUCCEEDED(endHr)) {
          (void)CollapseSelectionToRangeOffset(ec, pic, range, cursorOffset);
        }
        hr = endHr;
      }
      range->Release();
    }
    ClearCompositionBufferState();
    ReleaseCompositionObjects();
    return hr;
  }

  // Fallback: use InsertTextAtSelection if composition approach fails.
  const HRESULT fallbackHr = insertAtSelectionOnly();
  if (FAILED(fallbackHr)) {
    RecordCompatibilityFallback(L"InsertTextAtSelection", fallbackHr);
  }
  return fallbackHr;
}

HRESULT CSrfTip::CommitReadingText(TfEditCookie ec, ITfContext* pic) {
  if (m_reading.empty()) return S_OK;
  const std::wstring text = m_reading;
  ITfContext* ctx = m_pCompositionContext ? m_pCompositionContext : (pic ? pic : m_pFocusContext);
  if (!ctx) return E_INVALIDARG;

  ctx->AddRef();
  const HRESULT hr = CommitDirectText(ec, ctx, text);
  ctx->Release();
  if (SUCCEEDED(hr)) {
    ClearCompositionBufferState();
    ReleaseCompositionObjects();
  }
  return hr;
}

HRESULT CSrfTip::CommitReadingThenDirectText(TfEditCookie ec, ITfContext* pic, const std::wstring& text) {
  return CommitReadingThenDirectTextWithCursor(ec, pic, text, -1);
}

HRESULT CSrfTip::CommitReadingThenDirectTextWithCursor(TfEditCookie ec, ITfContext* pic,
                                                       const std::wstring& text,
                                                       LONG cursorOffset) {
  HRESULT hr = S_OK;
  if (!m_reading.empty()) {
    hr = CommitReadingText(ec, pic);
    if (FAILED(hr)) return hr;
  }
  // CommitCandidate may have triggered single-char continuation, starting a new
  // composition with remaining syllables. Cancel and release it so the punctuation
  // is inserted cleanly via CommitDirectText's composition path.
  if (m_pComposition) {
    CancelCompositionEdit(ec);
    ReleaseCompositionObjects();
  }
  if (text.empty()) return S_OK;
  return CommitDirectTextWithCursor(ec, pic, text, cursorOffset);
}

HRESULT CSrfTip::ApplyCompositionDisplayAttribute(TfEditCookie ec, ITfContext* pic) {
  if (!pic || !m_pComposition) return E_FAIL;
  EnsureDisplayAttributeAtom();
  if (m_displayAttrAtom == TF_INVALID_GUIDATOM) return S_OK;

  ITfProperty* property = nullptr;
  HRESULT hr = pic->GetProperty(GUID_PROP_ATTRIBUTE, &property);
  if (FAILED(hr) || !property) return FAILED(hr) ? hr : E_FAIL;

  ITfRange* range = nullptr;
  hr = m_pComposition->GetRange(&range);
  if (SUCCEEDED(hr) && range) {
    VARIANT value;
    VariantInit(&value);
    value.vt = VT_I4;
    value.lVal = m_displayAttrAtom;
    hr = property->SetValue(ec, range, &value);
    range->Release();
  }

  property->Release();
  return hr;
}

HRESULT CSrfTip::SetCompositionSelection(TfEditCookie ec, ITfContext* pic) {
  if (!pic || !m_pComposition) return E_FAIL;

  ITfRange* range = nullptr;
  HRESULT hr = m_pComposition->GetRange(&range);
  if (FAILED(hr) || !range) return FAILED(hr) ? hr : E_FAIL;

  ITfRange* selectionRange = nullptr;
  hr = range->Clone(&selectionRange);
  range->Release();
  if (FAILED(hr) || !selectionRange) return FAILED(hr) ? hr : E_FAIL;

  const LONG cursor = static_cast<LONG>(std::min(m_readingCursor, m_reading.size()));
  if (cursor > 0) {
    LONG shifted = 0;
    (void)selectionRange->ShiftStart(ec, cursor, &shifted, nullptr);
  }
  (void)selectionRange->Collapse(ec, TF_ANCHOR_START);

  TF_SELECTION selection = {};
  selection.range = selectionRange;
  selection.style.ase = TF_AE_NONE;
  selection.style.fInterimChar = FALSE;
  hr = pic->SetSelection(ec, 1, &selection);
  selectionRange->Release();
  return hr;
}
