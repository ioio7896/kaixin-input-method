#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <msctf.h>
#include <ctffunc.h>

#include <condition_variable>
#include <cstdint>
#include <filesystem>
#include <atomic>
#include <mutex>
#include <string>
#include <thread>
#include <utility>
#include <unordered_set>
#include <vector>

#include "candidate_window.h"
#include "ime_model.h"
#include "notification_window.h"
#include "pinyin_stub.h"

struct CKeyEventSink;
struct CThreadMgrEventSink;
class CCompositionSink;
class CSrfCandidateListUIElement;

struct SrfFocusSnapshot {
  uintptr_t contextCookie = 0;
  HWND hwnd = nullptr;
  DWORD processId = 0;
  std::wstring processName;
  uint64_t generation = 0;
};

class CSrfTip : public ITfTextInputProcessorEx,
                 public ITfDisplayAttributeProvider,
                 public ITfFunctionProvider,
                 public ITfFnConfigure {
  LONG m_cRef = 1;

  ITfThreadMgr* m_pThreadMgr = nullptr;
  ITfKeystrokeMgr* m_pKeystrokeMgr = nullptr;
  ITfSource* m_pSource = nullptr;
  ITfCompartmentMgr* m_pCompartmentMgr = nullptr;
  TfClientId m_tid = 0;

  DWORD m_dwThreadMgrSinkCookie = TF_INVALID_COOKIE;
  TfGuidAtom m_displayAttrAtom = TF_INVALID_GUIDATOM;

  CKeyEventSink* m_pKeySink = nullptr;
  CThreadMgrEventSink* m_pThreadMgrSink = nullptr;
  CCompositionSink* m_pCompSink = nullptr;

  ITfContext* m_pFocusContext = nullptr;

  ITfComposition* m_pComposition = nullptr;
  ITfContext* m_pCompositionContext = nullptr;

  std::wstring m_reading;
  size_t m_readingCursor = 0;
  std::vector<std::wstring> m_candidates;
  std::vector<std::wstring> m_candidateMeta;
  std::wstring m_candidatesReading;
  enum class SrfCandidateViewState : UINT {
    Empty = 0,
    Stable = 1,
    Pending = 2,
    Stale = 3,
    PartialCurrent = 4,
  };
  SrfCandidateViewState m_candidateViewState = SrfCandidateViewState::Empty;
  ULONGLONG m_candidateViewStateSinceTick = 0;
  UINT m_candSel = 0;
  UINT m_candPage = 0;
  UINT m_lastSyncedCandSel = 0;
  UINT m_lastSyncedCandPage = 0;
  unsigned long long m_candidateInteractionVersion = 0;
  unsigned long long m_partialCandidateInteractionVersion = 0;
  std::wstring m_partialCandidateInteractionReading;
  RECT m_lastCandidateRect = {};
  bool m_hasLastCandidateRect = false;
  // 逐字组词提交首字时，保留当前锚点直到“剩余拼音”的首批候选落地，
  // 避免候选窗先消失、再因新 composition 的临时坐标跳位。
  std::wstring m_preserveCandidateAnchorReading;
  RECT m_stickyCandidateRect = {};
  bool m_hasStickyCandidateRect = false;
  UINT m_stickyCandidateAnchorQuality = 0;
  std::wstring m_lastCandidateAnchorSource;
  UINT m_lastCandidateAnchorQuality = 0;
  ULONGLONG m_lastCandidateAnchorSourceSwitchTick = 0;
  unsigned int m_candidateAnchorSourceSwitchCount = 0;

  // 逐字拼词（单字上屏后保留剩余拼音）时，累计用户造的新词。
  // 目的：即使词库没有该多字词，用户也能逐字拼出，并在结束时把整词写入用户词典。
  bool m_userPhraseComposeActive = false;
  bool m_userPhraseComposeValid = false;
  std::wstring m_userPhraseComposeOriginalReading;
  std::wstring m_userPhraseComposeCommitted;

  /// 在 CommitCandidate 复杂分支中保存/恢复逐字拼词状态，避免手动恢复遗漏。
  class SrfPhraseComposeStateGuard {
   public:
    explicit SrfPhraseComposeStateGuard(CSrfTip* tip);
    ~SrfPhraseComposeStateGuard();
    SrfPhraseComposeStateGuard(const SrfPhraseComposeStateGuard&) = delete;
    SrfPhraseComposeStateGuard& operator=(const SrfPhraseComposeStateGuard&) = delete;

   private:
    CSrfTip* tip_;
    bool wasActive_;
    bool wasValid_;
    std::wstring originalReading_;
    std::wstring committed_;
  };

  bool m_imeOpen = true;
  bool m_fullShape = false;
  bool m_cnPunct = true;
  bool m_fuzzyPinyin = false;
  bool m_doublePinyin = false;
  bool m_traditionalOutput = false;
  bool m_manualGameCompatActive = false;
  bool m_manualAsciiModeActive = false;
  // 手动兼容状态只对触发它的前台窗口有效；bypass 是“恢复中文”的临时覆盖。
  bool m_manualCompatibilityBypass = false;
  HWND m_manualModeHwnd = nullptr;
  DWORD m_manualModeProcessId = 0;
  std::wstring m_manualModeAppName;
  bool m_nextSingleQuoteOpen = true;
  bool m_nextDoubleQuoteOpen = true;
  bool m_cuasWorkaroundEnabled = false;
  bool m_uiLessMode = false;
  bool m_fullscreenCompatActive = false;
  bool m_gameCompatActive = false;
  bool m_configuredGameCompatActive = false;
  bool m_builtinGameCompatActive = false;
  bool m_sensitiveInputActive = false;
  HWND m_compatibilityHwnd = nullptr;
  DWORD m_compatibilityProcessId = 0;
  std::wstring m_compatibilityAppName;
  SrfFullscreenPolicy m_lastCompatibilityPolicy = SrfFullscreenPolicy::Off;
  bool m_hasLastCompatibilityPolicy = false;
  bool m_compatibilityAsciiCleanupPending = false;
  HWND m_fullscreenCompatCandidateHwnd = nullptr;
  ULONGLONG m_fullscreenCompatCandidateSince = 0;
  ULONGLONG m_compatLastRawHitTick = 0;
  bool m_runtimeHideUiFallbackActive = false;
  std::wstring m_runtimeHideUiFallbackAppName;
  bool m_runtimeAsciiFallbackActive = false;
  bool m_shiftTapActive = false;
  bool m_shiftTapUsedWithOtherKey = false;
  ULONGLONG m_shiftTapStartTick = 0;
  ULONGLONG m_lastActivationTick = 0;
  bool m_ignoreImeToggleUntilModifiersReleased = false;
  /// 每次请求失焦取消时递增；异步 EditSession 完成时若与当前值不一致则丢弃（避免乱序重复取消）。
  uint64_t m_focusCancelSequence = 0;
  uint64_t m_compositionGeneration = 0;
  uint64_t m_focusGeneration = 0;

  SrfContext m_context = {};
  SrfStatus m_status = {};
  SrfUIStyle m_uiStyle = {};
  SrfConfig m_config = {};
  std::wstring m_activeAppName;
  std::wstring m_runtimeAsciiFallbackAppName;
  std::wstring m_lastCompatibilityLogKey;
  std::unordered_set<std::wstring> m_locallyPinnedCandidateKeys;
  HWND m_cachedFocusedHwnd = nullptr;
  DWORD m_cachedFocusedProcessId = 0;
  std::wstring m_cachedFocusedProcessName;
  uint64_t m_loadedConfigVersion = 0;
  bool m_configReloadPending = false;
  TF_PRESERVEDKEY m_registeredScreenshotKey = {};
  bool m_hasRegisteredScreenshotKey = false;

  CSrfCandidateListUIElement* m_candidateUi = nullptr;
  CNotificationWindow m_notificationWindow;

 public:
  CSrfTip();
  ~CSrfTip();
  CSrfTip(const CSrfTip&) = delete;
  CSrfTip& operator=(const CSrfTip&) = delete;

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;

  STDMETHODIMP Activate(ITfThreadMgr* ptim, TfClientId tid) override;
  STDMETHODIMP Deactivate() override;
  STDMETHODIMP ActivateEx(ITfThreadMgr* ptim, TfClientId tid, DWORD dwFlags) override;

  STDMETHODIMP EnumDisplayAttributeInfo(IEnumTfDisplayAttributeInfo** ppEnum) override;
  STDMETHODIMP GetDisplayAttributeInfo(REFGUID guid, ITfDisplayAttributeInfo** ppInfo) override;

  STDMETHODIMP GetType(GUID* pguid) override;
  STDMETHODIMP GetDescription(BSTR* pbstrDesc) override;
  STDMETHODIMP GetFunction(REFGUID rguid, REFIID riid, IUnknown** ppunk) override;
  STDMETHODIMP GetDisplayName(BSTR* pbstrName) override;
  STDMETHODIMP Show(HWND hwndParent, LANGID langid, REFGUID rguidProfile) override;

  HRESULT ProcessKey(TfEditCookie ec, ITfContext* pic, UINT vk, LPARAM lParam,
                     bool shiftDown, bool* pHandled);
  bool WouldEatKey(UINT vk);
  HRESULT CommitCandidate(TfEditCookie ec, size_t idx);
  HRESULT CommitCandidateSnapshot(TfEditCookie ec, ITfContext* requestContext, size_t idx,
                                   const std::wstring& reading,
                                   const std::wstring& committedText,
                                   const std::wstring& metaText,
                                   const std::vector<std::wstring>& skippedCandidates);
  HRESULT CommitReadingText(TfEditCookie ec, ITfContext* pic);
  UINT CandidatePageSize() const;
  std::wstring CandidateBarMainTitle() const;
  std::vector<std::wstring> CandidateBarModeTags() const;

  void SetFocusContext(ITfContext* pic);
  void ApplyAppOptionsForFocusedContext(bool showNotification);
  void RequestCancelCompositionOnFocusLoss();
  void HandleFocusLossCancelEditSession(TfEditCookie ec, uint64_t generation, uint64_t cancelSequence);
  void RequestCompatibilityAsciiCleanup();
  void HandleCompatibilityAsciiCleanupEditSession(TfEditCookie ec);
  void CancelCompositionEdit(TfEditCookie ec);

  void OnCandidateClicked(UINT indexInPage);
  void OnCandidateWheel(int wheelDelta);
  void ApplyCandidatePinChoice(size_t idx, bool pinned);
  void ApplyCandidateMenuCommand(size_t idx, int command);
  void ToggleCandidatePinChoice(size_t idx);
  bool IsCandidatePinned(size_t idx) const;
  bool IsClipboardQuickCandidate(size_t idx) const;
  bool IsClipboardCandidatePinned(size_t idx) const;
  bool ClipboardCandidatePageInfo(size_t idx, UINT* page, UINT* pages) const;
  std::wstring CandidateSourceDescription(size_t idx) const;

  friend struct CKeyEventSink;
  friend struct CThreadMgrEventSink;
  friend class CCompositionSink;
  friend class CSrfCandidateListUIElement;
  friend class CEditSessionDeferredRefresh;
  friend class CEditSessionApplyAsyncCandidates;
  friend class CEditSessionCandidateAnchorRefresh;

 private:
  HRESULT _UnadviseSinks();
  HRESULT RegisterPreservedKeys();
  void UnregisterPreservedKeys();
  void ClearCompositionBufferState();
  void ClearFocusBoundCandidateState(const wchar_t* reason);
  void ReleaseCompositionObjects();
  void ReleaseCompositionState();
  void RefreshCandidates();
  bool RefreshCandidatesAsync();
  bool ApplyCandidateRefreshResult(const std::wstring& reading,
                                   std::vector<std::wstring> nextCandidates,
                                   std::vector<std::wstring> nextMeta,
                                   SrfEngineState stateAfterLookup,
                                   SrfLookupCandidatesStatus lookupStatus,
                                   bool asyncResult,
                                   unsigned long long requestId);
  bool TryApplyPrefixCandidatePlaceholder(const std::wstring& reading,
                                          SrfEngineState stateBefore,
                                          unsigned long long requestId);
  void RememberPrefixCandidateCache(const std::wstring& reading);
  void ApplyAsyncCandidateResult(TfEditCookie ec);
  bool CurrentCandidatesPartial() const;
  const wchar_t* CandidateViewStateName() const;
  bool CandidateViewInteractive() const;
  bool CandidateViewPendingVisual() const;
  void SetCandidateViewState(SrfCandidateViewState state, const wchar_t* reason);
  UINT MaxCandidatePage() const;
  void ClampCandidateState();
  void UpdateCandidateWindow(TfEditCookie ec);
  HWND CandidateOverlayTargetWindow() const;
  bool CandidateGameOverlayActive() const;
  bool FullscreenCandidateOverlayActive() const;
  void RefreshCandidateWindowEnvironment();
  bool RequestCandidateWindowAnchorRefresh();
  void OnCandidateWindowAnchorRefreshTimer();
  void ScheduleCandidateWindowAnchorRefreshRetry();
  void CancelCandidateWindowAnchorRefreshRetry();
  void RedrawCandidateUi();
  void RedrawCandidateUiImmediate();
  void RedrawCandidateUiNow();
  void ClampReadingCursor();
  HRESULT EnsureComposition(TfEditCookie ec, ITfContext* pic);
  HRESULT SyncCompositionText(TfEditCookie ec, ITfContext* pic, bool refreshCandidates);
  HRESULT CommitDirectText(TfEditCookie ec, ITfContext* pic, const std::wstring& text);
  HRESULT CommitDirectTextWithCursor(TfEditCookie ec, ITfContext* pic, const std::wstring& text,
                                     LONG cursorOffset);
  HRESULT CommitReadingThenDirectText(TfEditCookie ec, ITfContext* pic, const std::wstring& text);
  HRESULT CommitReadingThenDirectTextWithCursor(TfEditCookie ec, ITfContext* pic,
                                                const std::wstring& text, LONG cursorOffset);
  HRESULT ApplyCompositionDisplayAttribute(TfEditCookie ec, ITfContext* pic);
  HRESULT SetCompositionSelection(TfEditCookie ec, ITfContext* pic);
  void EnsureDisplayAttributeAtom();
  void LoadCompartmentState();
  void SyncCompartmentState();
  void ApplyRustModeFlags();
  DWORD RustModeFlags() const;
  DWORD GetCompartmentDWORD(REFGUID guid, DWORD fallback) const;
  void SetCompartmentDWORD(REFGUID guid, DWORD value);
  void ApplyDefaultPunctuationForImeMode();
  void BeginPreservedKeyGuardAfterActivation();
  void LogActivationEnvironment(DWORD dwFlags);
  bool ShouldSuppressImeTogglePreservedKey();
  void ToggleImeOpen();
  void ToggleFullShape();
  void ToggleChinesePunctuation();
  void ToggleFuzzyPinyin();
  void ToggleDoublePinyin();
  void ToggleTraditionalOutput();
  void ToggleManualGameCompat();
  void ToggleManualAsciiMode(TfEditCookie ec);
  void CaptureManualModeOwner();
  bool ReconcileManualModeOwner();
  void ClearManualModeOwner();
  void RestoreImeModeFromCurrentAppOptions();
  bool IsConfiguredHotkey(UINT vk, const SrfHotkeyOptions& hotkey) const;
  void LearnCommittedText(const std::wstring& committedText);
  HRESULT CommitCandidateResolved(TfEditCookie ec, ITfContext* requestContext, size_t idx,
                                   const std::wstring* snapshotReading,
                                   const std::wstring* snapshotCommitted,
                                   const std::wstring* snapshotMeta,
                                   const std::vector<std::wstring>* snapshotSkippedCandidates);
  HRESULT RequestCommitCandidate(size_t idx);
  HRESULT RequestCommitReadingText();
  std::wstring TranslateDirectKey(UINT vk, LPARAM lParam, bool shiftDown) const;
  std::wstring ConvertDirectText(std::wstring text);
  std::wstring ConvertDirectTextWithCompletion(std::wstring text, LONG* cursorOffset);
  bool ShouldHandleDirectKey(UINT vk) const;
  bool ShouldUseTemporaryEnglish(UINT vk, bool shiftDown) const;
  std::wstring BuildCompositionDisplay() const;
  std::wstring FormatCandidateDisplayText(size_t index) const;
  const std::vector<std::wstring>& BuildCandidateDisplayItems() const;
  unsigned long long CandidateDisplayVersion() const { return m_candidateDisplayVersion; }
  bool CurrentCandidatesClipboardQuickMode() const;
  bool CurrentCandidatesForceVerticalLayout() const;
  SrfUIStyle EffectiveCandidateUiStyle() const;
  CandidatePageLayoutMetrics BuildCandidatePageLayout() const;
  void InvalidateCandidatePageLayoutCache();
  UINT CandidatePageCount() const;
  UINT CandidatePageStart(UINT page) const;
  UINT CandidatePageEndExclusive(UINT page) const;
  UINT CandidatePageForIndex(UINT index) const;
  UINT CandidateIndexInPage(UINT index) const;
  void LoadConfiguration();
  bool ReloadConfigurationIfChanged();
  bool HasConfigurationChanged() const;
  void ApplyRuntimeConfigReload();
  void ApplyPendingRuntimeConfigIfSafe();
  SrfUIStyle ResolveUiStyleForApp(const std::wstring& appName) const;
  SrfFocusPolicy EffectiveFocusPolicy() const;
  const wchar_t* EffectiveFocusPolicyName() const;
  void RefreshRuntimeConfig();
  void RefreshCompatibilityState();
  const std::wstring& CompatibilityAppName() const;
  SrfFullscreenPolicy EffectiveCompatibilityPolicy() const;
  const wchar_t* EffectiveInputModeSource() const;
  const wchar_t* EffectiveCompatibilityPolicyName() const;
  SrfOverlayBackend EffectiveCandidateOverlayBackend() const;
  bool ShouldUseExternalCandidateOverlay() const;
  SrfCommitTransport EffectiveCommitTransport() const;
  const wchar_t* EffectiveCommitTransportName() const;
  bool ShouldHideUiForCompatibility() const;
  bool ShouldForceAsciiForCompatibility() const;
  bool ShouldUseBlindCommitForCompatibility() const;
  bool EnsureBlindCommitCandidatesReady();
  bool ShouldSuppressLearningForPrivacy() const;
  bool ShouldSuppressClipboardForPrivacy() const;
  bool ShouldSuppressCandidatesForPrivacy() const;
  void RecordCompatibilityUiFallback(const wchar_t* reason, HRESULT hr);
  void RecordCompatibilityFallback(const wchar_t* apiName, HRESULT hr);
  void SyncStatusModel();
  void SyncCandidateContextState(const CandidatePageLayoutMetrics* layout = nullptr);
  void RebuildContextModel();
  std::wstring FocusedProcessName();
  SrfFocusSnapshot CaptureFocusSnapshot(ITfContext* context) const;
  bool FocusSnapshotMatches(const SrfFocusSnapshot& snapshot) const;
  std::wstring FormatFocusSnapshotForLog(const SrfFocusSnapshot& snapshot) const;
  void EnsureTrayHelperRunningAsync();
  void ShowNotification(SrfNotificationKind kind, const std::wstring& text);
  void MaybeNotifyEngineHealth();
  void MoveReadingCaretBySyllable(bool forward);
  bool LoadGlobalAsciiState() const;
  bool TryLoadGlobalAsciiState(bool* asciiMode) const;
  void SaveGlobalAsciiState(bool asciiMode) const;
  void ApplyGlobalAsciiStateFromRegistry();
  void RefreshKeyHotPathState();
  void RefreshCompatibilityStateThrottled(bool force = false);
  void InvalidateHotPathStateCache();
  void ScheduleDeferredCandidateRefresh();
  void ScheduleDeferredCandidateRefresh(DWORD delayMs);
  void CancelDeferredCandidateRefresh();
  void OnLearnCommitCompleted(unsigned long long requestId, bool succeeded);
  void OnDeferredCandidateRefreshTimer();
  void ScheduleCandidateUiRedraw(DWORD delayMs);
  void CancelScheduledCandidateUiRedraw();
  void OnCandidateUiRedrawTimer();
  bool ShouldDeferFocusContextClear() const;
  bool ScheduleDeferredFocusContextClear();
  void CancelDeferredFocusContextClear();
  void OnDeferredFocusContextClearTimer();
  bool EnsureDeferredTimerWindow();
  void OnAsyncCandidateResultMessage();
  bool EnsureCandidateLookupWorker();
  void StopCandidateLookupWorker();
  void QueueCandidateLookup(const std::wstring& reading, unsigned long long serial,
                            unsigned long long engineRequestId, HWND hwnd,
                            const SrfFocusSnapshot& focus);
  void CandidateLookupWorkerMain();

  bool m_engineHealthNotifiedThisComposition = false;
  /// 延迟候选刷新：当 try_lock 失败或引擎预热中时，用定时器重试。
  HWND m_deferredTimerHwnd = nullptr;
  struct PendingLearnNotification {
    unsigned long long requestId = 0;
    std::wstring reading;
    std::wstring phrase;
  };
  std::vector<PendingLearnNotification> m_pendingLearnNotifications;
  bool m_deferredRefreshPending = false;
  std::wstring m_deferredRefreshReading;
  ULONGLONG m_deferredRefreshDueTick = 0;
  bool m_candidateUiRedrawPending = false;
  ULONGLONG m_candidateUiRedrawDueTick = 0;
  ULONGLONG m_candidateUiTransientMissingSince = 0;
  bool m_deferredFocusClearPending = false;
  ULONGLONG m_deferredFocusClearDueTick = 0;
  SrfFocusSnapshot m_deferredFocusClearSnapshot = {};
  bool m_candidateAnchorRefreshEditPending = false;
  ULONGLONG m_candidateAnchorRefreshRequestTick = 0;
  bool m_candidateAnchorRefreshRetryPending = false;
  std::atomic<unsigned long long> m_candidateLookupSerial{0};
  std::atomic<ULONGLONG> m_candidateLookupBackpressureUntilTick{0};
  std::mutex m_asyncCandidateMutex;
  bool m_asyncCandidatePending = false;
  unsigned long long m_asyncCandidatePendingSerial = 0;
  std::wstring m_asyncCandidatePendingReading;
  std::vector<std::wstring> m_asyncCandidatePendingItems;
  std::vector<std::wstring> m_asyncCandidatePendingMeta;
  SrfEngineState m_asyncCandidatePendingEngineState = SrfEngineState::Idle;
  SrfLookupCandidatesStatus m_asyncCandidatePendingLookupStatus =
      SrfLookupCandidatesStatus::Ok;
  ULONGLONG m_asyncCandidatePendingRequestTick = 0;
  SrfFocusSnapshot m_asyncCandidatePendingFocus = {};
  std::mutex m_candidateWorkerMutex;
  std::condition_variable m_candidateWorkerCv;
  std::thread m_candidateWorkerThread;
  bool m_candidateWorkerStop = false;
  bool m_candidateWorkerHasRequest = false;
  unsigned long long m_candidateWorkerRequestSerial = 0;
  unsigned long long m_candidateWorkerEngineRequestId = 0;
  std::wstring m_candidateWorkerRequestReading;
  HWND m_candidateWorkerNotifyHwnd = nullptr;
  ULONGLONG m_candidateWorkerRequestTick = 0;
  unsigned int m_candidateWorkerRapidRequestCount = 0;
  SrfFocusSnapshot m_candidateWorkerRequestFocus = {};
  ULONGLONG m_lastKeyHotPathRefreshTick = 0;
  ULONGLONG m_lastCompatibilityRefreshTick = 0;
  mutable bool m_candidatePageLayoutCacheValid = false;
  mutable CandidatePageLayoutMetrics m_candidatePageLayoutCache = {};
  mutable bool m_candidateDisplayItemsCacheValid = false;
  mutable std::vector<std::wstring> m_candidateDisplayItemsCache;
  unsigned long long m_candidateDisplayVersion = 0;
  mutable std::vector<std::wstring> m_lastCandidateModeTags;
  mutable ULONGLONG m_candidateModeTagsVisibleUntilTick = 0;
  struct PrefixCandidateCacheEntry {
    std::wstring reading;
    std::vector<std::wstring> candidates;
    std::vector<std::wstring> meta;
    ULONGLONG tick = 0;
  };
  std::vector<PrefixCandidateCacheEntry> m_prefixCandidateCache;
  static constexpr UINT_PTR kDeferredCandidateTimerId = 42;
  static constexpr UINT_PTR kCandidateUiRedrawTimerId = 44;
  static constexpr UINT_PTR kDeferredFocusClearTimerId = 45;
  static constexpr UINT_PTR kExternalOverlayHealthTimerId = 46;
  static constexpr UINT_PTR kCandidateAnchorRefreshTimerId = 47;
  static constexpr DWORD kDeferredCandidateRetryMs = 24;
  static constexpr DWORD kCandidateUiRedrawCoalesceMs = 4;
  static constexpr DWORD kCandidateUiTransientHideGraceMs = 140;
  static constexpr DWORD kExternalOverlayHealthIntervalMs = 500;
  static constexpr DWORD kCandidateAnchorRefreshRetryMs = 80;
  static constexpr DWORD kCandidateAnchorRefreshStaleMs = 1500;
  static constexpr DWORD kTransientFocusLossGraceMs = kCandidateUiTransientHideGraceMs;
  static constexpr UINT kAsyncCandidateResultMessage = WM_APP + 43;
  static constexpr UINT kLearnCommitCompletedMessage = WM_APP + 44;
  static constexpr DWORD kCandidateLookupCoalesceMs = 1;
  static constexpr DWORD kCandidateLookupWarmCoalesceMs = 3;
  static constexpr DWORD kCandidateLookupRapidCoalesceMs = 6;
  static constexpr DWORD kCandidateLookupBackpressureCoalesceMs = 8;
  static constexpr DWORD kCandidateLookupRapidInputMs = 40;
  static constexpr DWORD kCandidateLookupBackpressureMs = 120;
  static constexpr DWORD kDeferredCandidateIdleRetryMs = 120;
  static constexpr ULONGLONG kKeyHotPathRefreshMs = 40;
  static constexpr size_t kPrefixCandidateCacheCapacity = 24;
  static constexpr ULONGLONG kPrefixCandidateCacheTtlMs = 8000;
  static constexpr ULONGLONG kCompatibilityRefreshMs = 40;
  static constexpr ULONGLONG kFullscreenCompatStableMs = 250;
  static constexpr ULONGLONG kCompatibilityReleaseDebounceMs = 450;
  static LRESULT CALLBACK DeferredTimerWndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam);
};
