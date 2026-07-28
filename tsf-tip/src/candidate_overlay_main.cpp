#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <algorithm>
#include <cstdint>
#include <cstring>
#include <cwchar>
#include <cwctype>
#include <limits>
#include <string>

#include <shellapi.h>

#include "candidate_overlay_protocol.h"
#include "candidate_overlay_placement.h"
#include "candidate_window.h"
#include "ime_config.h"
#include "ime_model.h"

namespace {

constexpr UINT_PTR kOverlayWatchTimerId = 1;
constexpr UINT kOverlayWatchIntervalMs = 250;
constexpr ULONGLONG kOverlayHiddenIdleExitMs = 60000;
constexpr UINT kOverlayApplySnapshotMessage = WM_APP + 0x31;
constexpr UINT_PTR kCandidatePreviewTimerId = 0x4B585052u;  // "KXPR"
constexpr UINT kCandidatePreviewDurationMs = 10000;

bool CandidatePreviewRequested() {
  int count = 0;
  wchar_t** args = CommandLineToArgvW(GetCommandLineW(), &count);
  if (!args) return false;
  const bool requested = count == 2 && wcscmp(args[1], L"--candidate-preview") == 0;
  LocalFree(args);
  return requested;
}

int RunCandidatePreview() {
  SrfConfig config = LoadSrfConfig();
  SrfUIStyle style = config.style;
  style.candidateTopmost = true;
  style.candidateLeftClick = false;
  style.candidateRightClick = false;

  POINT cursor = {};
  GetCursorPos(&cursor);
  RECT anchor = {cursor.x, cursor.y, cursor.x + 1, cursor.y + 24};
  const std::vector<std::wstring> items = {
      L"你好", L"拟好", L"你号", L"霓虹", L"你好呀"};
  std::wstring selectedComment;
  auto appendSelectedDetail = [&](const wchar_t* detail) {
    if (!detail || !*detail) return;
    if (!selectedComment.empty()) selectedComment += L"  ·  ";
    selectedComment += detail;
  };
  if (style.showCandidateReading) appendSelectedDetail(L"ni hao");
  if (style.showCandidateSource) appendSelectedDetail(L"系统词");
  if (style.showCandidateScore) appendSelectedDetail(L"0.982");
  const std::vector<std::wstring> comments = {selectedComment, L"", L"", L"", L""};
  const std::vector<std::wstring> labels = {L"1", L"2", L"3", L"4", L"5"};
  const std::vector<std::wstring> modeTags =
      style.showModeInCandidateHeader ? std::vector<std::wstring>{L"中", L"中标"}
                                      : std::vector<std::wstring>{};

  CCandidateWindow window;
  window.SetGameOverlay(false, false, nullptr);
  window.SetStyle(style);
  window.PrepareResources();
  window.Show(L"ni hao", items, comments, labels, {}, {}, 0, 2, 0, anchor,
              modeTags, false, false);

  const UINT_PTR timer = SetTimer(nullptr, kCandidatePreviewTimerId,
                                  kCandidatePreviewDurationMs, nullptr);
  if (!timer) {
    window.Destroy();
    return 2;
  }
  MSG message = {};
  while (GetMessageW(&message, nullptr, 0, 0) > 0) {
    if (message.message == WM_TIMER && message.wParam == timer) break;
    TranslateMessage(&message);
    DispatchMessageW(&message);
  }
  KillTimer(nullptr, timer);
  window.Destroy();
  return 0;
}

HWND RootOverlayWindow(HWND hwnd) {
  if (!hwnd || !IsWindow(hwnd)) return nullptr;
  HWND root = GetAncestor(hwnd, GA_ROOT);
  return root && IsWindow(root) ? root : hwnd;
}

bool TargetIsForeground(HWND targetHwnd, DWORD processId) {
  const HWND foreground = GetForegroundWindow();
  if (!foreground || !targetHwnd || processId == 0) return false;
  DWORD foregroundProcessId = 0;
  GetWindowThreadProcessId(foreground, &foregroundProcessId);
  return foregroundProcessId == processId &&
         RootOverlayWindow(foreground) == RootOverlayWindow(targetHwnd);
}

bool TargetWindowMatches(HWND hwnd, DWORD processId) {
  if (!hwnd || !IsWindow(hwnd) || processId == 0) return false;
  DWORD actualProcessId = 0;
  GetWindowThreadProcessId(hwnd, &actualProcessId);
  return actualProcessId == processId;
}

bool WindowHasClass(HWND hwnd, const wchar_t* expectedClass) {
  if (!hwnd || !expectedClass) return false;
  wchar_t actualClass[128] = {};
  return GetClassNameW(hwnd, actualClass, _countof(actualClass)) > 0 &&
         wcscmp(actualClass, expectedClass) == 0;
}

void ApplyCompactGameStyle(SrfUIStyle* style, bool forceHorizontal = true) {
  if (!style) return;
  if (forceHorizontal) {
    style->candidateHorizontal = true;
    style->candidateHorizontalCompact = true;
  }
  style->candidatePageSize = std::min(style->candidatePageSize, 5u);
  style->candidateHorizontalCount = std::min(style->candidateHorizontalCount, 5u);
  style->candidateTopmost = true;
  style->candidateOpacity = 100;
  style->candidateMaterial = SrfCandidateMaterial::Solid;
  style->candidateDensity = SrfCandidateDensity::Compact;
  style->candidateLayoutVariant = SrfCandidateLayoutVariant::Compact;
  style->candidateLeftClick = false;
  style->candidateRightClick = false;
  style->showCandidateReading = false;
  style->showCandidateScore = false;
  style->showCandidateSource = false;
  style->showModeInCandidateHeader = false;
}

SrfUIStyle ResolveOverlayStyle(const SrfConfig& config,
                               const SrfCandidateOverlaySnapshot& snapshot) {
  SrfUIStyle style = config.style;
  if (const SrfAppOptions* options = FindAppOptions(config, snapshot.appPath)) {
    if (options->hasInlinePreedit) style.inlinePreedit = options->inlinePreedit;
    if (options->hasEnhancedPosition) style.enhancedPosition = options->enhancedPosition;
    if (options->hasCandidateTopmost) style.candidateTopmost = options->candidateTopmost;
    if (options->hasOverlayScale) {
      style.candidateScalePercent =
          std::clamp(options->overlayScalePercent, 50u, 200u);
    }
    if (options->hasOverlayAnchor) {
      style.candidateOverlayAnchor = options->overlayAnchor;
    }
    if (options->hasGameProfile && options->gameCompactProfile) {
      ApplyCompactGameStyle(&style);
    }
  }
  if (snapshot.gameCompact) ApplyCompactGameStyle(&style);
  if (snapshot.layoutResolved) {
    style.candidateHorizontal = snapshot.horizontalLayout;
    style.candidateHorizontalCompact =
        snapshot.horizontalLayout && snapshot.horizontalCompact;
  }
  style.candidateFullscreenPlacement = snapshot.fullscreenPlacement;
  // The external game overlay is intentionally read-only. It must never steal
  // a click or activate itself even if a stale config still enables mouse UI.
  style.candidateTopmost = true;
  style.candidateLeftClick = false;
  style.candidateRightClick = false;
  return style;
}

class CandidateOverlayHost : public ICandidateWindowEvents {
 public:
  ~CandidateOverlayHost() {
    if (clientProcess_) CloseHandle(clientProcess_);
  }

  bool Initialize(HINSTANCE instance, DWORD clientProcessId,
                  const std::wstring& controlToken,
                  const SrfCandidateOverlayAuthSecret& authSecret) {
    if (clientProcessId == 0 || controlToken.empty() || !authSecret.Valid()) {
      return false;
    }
    DWORD clientSessionId = 0;
    DWORD currentSessionId = 0;
    if (!ProcessIdToSessionId(clientProcessId, &clientSessionId) ||
        !ProcessIdToSessionId(GetCurrentProcessId(), &currentSessionId) ||
        clientSessionId != currentSessionId) {
      return false;
    }
    clientProcess_ = OpenProcess(SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
                                 FALSE, clientProcessId);
    if (!clientProcess_) return false;
    clientProcessId_ = clientProcessId;
    authSecret_ = authSecret;
    std::wstring imagePath(32768, L'\0');
    DWORD imageLength = static_cast<DWORD>(imagePath.size());
    if (QueryFullProcessImageNameW(clientProcess_, 0, imagePath.data(),
                                   &imageLength) && imageLength != 0) {
      imagePath.resize(imageLength);
      clientImagePath_ = std::move(imagePath);
    }

    instance_ = instance;
    candidateWindow_.SetEvents(this);
    // Do all potentially expensive font/config/render initialization before
    // publishing the control window. Discoverable now means ready to accept a
    // snapshot within the client's short input-thread timeout.
    PrepareResources();
    WNDCLASSEXW windowClass = {};
    windowClass.cbSize = sizeof(windowClass);
    windowClass.hInstance = instance_;
    windowClass.lpfnWndProc = &CandidateOverlayHost::WndProc;
    windowClass.lpszClassName = SrfCandidateOverlayControlWindowClass();
    if (!RegisterClassExW(&windowClass) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
      return false;
    }
    controlWindow_ = CreateWindowExW(
        0, SrfCandidateOverlayControlWindowClass(), controlToken.c_str(), 0, 0,
        0, 0, 0, HWND_MESSAGE, nullptr, instance_, this);
    if (!controlWindow_) return false;
    lastMessageTick_ = GetTickCount64();
    SetTimer(controlWindow_, kOverlayWatchTimerId, kOverlayWatchIntervalMs, nullptr);
    return true;
  }

  int Run() {
    MSG message = {};
    while (GetMessageW(&message, nullptr, 0, 0) > 0) {
      TranslateMessage(&message);
      DispatchMessageW(&message);
    }
    candidateWindow_.Destroy();
    return static_cast<int>(message.wParam);
  }

  void OnCandidateClicked(UINT) override {}
  void OnCandidateRightClicked(UINT, POINT) override {}
  void OnCandidatePinRequested(UINT, bool) override {}
  void OnCandidateMenuCommand(UINT, int) override {}
  void OnCandidateWheel(int) override {}
  void OnCandidateEnvironmentChanged() override {
    if (!visible_ || !TargetWindowMatches(activeTargetHwnd_, activeTargetProcessId_) ||
        !TargetIsForeground(activeTargetHwnd_, activeTargetProcessId_)) {
      const HWND sender = activeSenderHwnd_;
      Hide();
      NotifyStatusChanged(sender);
      return;
    }
    // A caret rectangle is owned by TSF and is expressed in the source
    // process' current coordinate space.  After Alt+Enter, a DPI transition,
    // or a monitor move the helper cannot safely transform the old rectangle.
    // Withdraw it and ask the source to sample a fresh caret instead.
    if (activeSnapshot_.caretAnchor) {
      const HWND sender = activeSenderHwnd_;
      Hide();
      NotifyStatusChanged(sender);
      return;
    }
    RefreshConfiguration();
    activeSnapshot_.fullscreenPlacement =
        IsCandidateOverlayTargetFullscreen(activeTargetHwnd_);
    const SrfAppOptions* options = FindAppOptions(config_, activeSnapshot_.appPath);
    RECT refreshedAnchor = {};
    if (ResolveCandidateGameOverlayAnchor(
            activeTargetHwnd_, activeSnapshot_.fullscreenPlacement, options,
            &refreshedAnchor)) {
      activeSnapshot_.anchor = refreshedAnchor;
    }
    ShowSnapshot(activeSnapshot_);
    if (!candidateWindow_.IsVisible()) {
      const HWND sender = activeSenderHwnd_;
      Hide();
      NotifyStatusChanged(sender);
    }
  }

 private:
  bool ValidateSender(HWND sender, const SrfCandidateOverlaySnapshot& snapshot) const {
    if (!sender || !IsWindow(sender) ||
        !WindowHasClass(sender, SrfCandidateOverlayClientWindowClass()) ||
        snapshot.sourceProcessId != clientProcessId_ ||
        snapshot.targetProcessId != clientProcessId_ || snapshot.ownerId == 0 ||
        snapshot.authSecret.high != authSecret_.high ||
        snapshot.authSecret.low != authSecret_.low) {
      return false;
    }
    DWORD senderProcessId = 0;
    GetWindowThreadProcessId(sender, &senderProcessId);
    if (senderProcessId != clientProcessId_) return false;
    if (snapshot.targetHwnd &&
        !TargetWindowMatches(snapshot.targetHwnd, clientProcessId_)) {
      return false;
    }
    if (snapshot.visible &&
        (!snapshot.layoutResolved || !snapshot.anchorPhysical)) {
      return false;
    }
    return true;
  }

  bool CanAcceptSequence(const SrfCandidateOverlaySnapshot& snapshot) const {
    if (snapshot.sequence == 0 || snapshot.sequence <= lastAcceptedSequence_) {
      return false;
    }
    if (snapshot.visible) {
      if (acceptedOwnerId_ == snapshot.ownerId &&
          snapshot.focusGeneration < acceptedFocusGeneration_) {
        return false;
      }
    } else {
      if (acceptedOwnerId_ == 0 || acceptedOwnerId_ != snapshot.ownerId ||
          snapshot.focusGeneration < acceptedFocusGeneration_) {
        return false;
      }
      if (snapshot.targetHwnd && acceptedTargetHwnd_ &&
          RootOverlayWindow(snapshot.targetHwnd) !=
              RootOverlayWindow(acceptedTargetHwnd_)) {
        return false;
      }
    }
    return true;
  }

  void CommitAcceptedSequence(const SrfCandidateOverlaySnapshot& snapshot) {
    if (snapshot.visible) {
      acceptedOwnerId_ = snapshot.ownerId;
      acceptedFocusGeneration_ = snapshot.focusGeneration;
      acceptedTargetHwnd_ = snapshot.targetHwnd;
    } else {
      acceptedFocusGeneration_ = snapshot.focusGeneration;
    }
    lastAcceptedOwnerId_ = snapshot.ownerId;
    lastAcceptedSequence_ = snapshot.sequence;
  }

  bool HandleSnapshot(HWND sender, const COPYDATASTRUCT* copy) {
    if (!copy || copy->dwData != kSrfCandidateOverlayCopyDataId || !copy->lpData ||
        copy->cbData < sizeof(SrfCandidateOverlayWireHeader) ||
        copy->cbData > kSrfCandidateOverlayMaxPacketBytes) {
      return false;
    }
    SrfCandidateOverlaySnapshot snapshot = {};
    if (!DeserializeCandidateOverlaySnapshot(copy->lpData, copy->cbData, &snapshot) ||
        !ValidateSender(sender, snapshot)) {
      return false;
    }
    if (snapshot.visible &&
        (!TargetWindowMatches(snapshot.targetHwnd, snapshot.targetProcessId) ||
         !TargetIsForeground(snapshot.targetHwnd, snapshot.targetProcessId))) {
      return false;
    }
    if (!CanAcceptSequence(snapshot)) return false;

    lastMessageTick_ = GetTickCount64();
    if (!applySnapshotPosted_) {
      if (!PostMessageW(controlWindow_, kOverlayApplySnapshotMessage, 0, 0)) {
        return false;
      }
      applySnapshotPosted_ = true;
    }
    HWND supersededSender = nullptr;
    if (snapshot.visible && activeOwnerId_ != 0 &&
        activeOwnerId_ != snapshot.ownerId) {
      supersededSender = activeSenderHwnd_;
      Hide();
    }
    CommitAcceptedSequence(snapshot);
    pendingSnapshot_ = std::move(snapshot);
    pendingSenderHwnd_ = sender;
    pendingSnapshotPresent_ = true;
    if (supersededSender && supersededSender != sender) {
      NotifyStatusChanged(supersededSender);
    }
    return true;
  }

  SrfCandidateOverlayStatus HandleStatusQuery(
      HWND sender, const COPYDATASTRUCT* copy) const {
    if (!copy || copy->dwData != kSrfCandidateOverlayStatusCopyDataId ||
        !copy->lpData || copy->cbData != sizeof(SrfCandidateOverlayStatusQuery)) {
      return SrfCandidateOverlayStatus::Unavailable;
    }
    SrfCandidateOverlayStatusQuery query = {};
    std::memcpy(&query, copy->lpData, sizeof(query));
    if (query.magic != kSrfCandidateOverlayStatusMagic ||
        query.version != kSrfCandidateOverlayVersion ||
        query.bytes != sizeof(query) ||
        query.sourceProcessId != clientProcessId_ || query.ownerId == 0 ||
        query.sequence == 0 ||
        query.authSecretHigh != authSecret_.high ||
        query.authSecretLow != authSecret_.low || !sender || !IsWindow(sender) ||
        !WindowHasClass(sender, SrfCandidateOverlayClientWindowClass())) {
      return SrfCandidateOverlayStatus::Unavailable;
    }
    DWORD senderProcessId = 0;
    GetWindowThreadProcessId(sender, &senderProcessId);
    if (senderProcessId != clientProcessId_) {
      return SrfCandidateOverlayStatus::Unavailable;
    }
    SrfCandidateOverlayStatusState state = {};
    state.queryOwnerId = query.ownerId;
    state.querySequence = query.sequence;
    state.lastAcceptedOwnerId = lastAcceptedOwnerId_;
    state.lastAcceptedSequence = lastAcceptedSequence_;
    if (pendingSnapshotPresent_) {
      state.pendingOwnerId = pendingSnapshot_.ownerId;
      state.pendingSequence = pendingSnapshot_.sequence;
    }
    state.activeOwnerId = activeOwnerId_;
    state.lastAppliedSequence = lastAppliedSequence_;
    state.activeWindowVisible =
        visible_ && candidateWindow_.IsVisible();
    return ResolveCandidateOverlayStatus(state);
  }

  LRESULT HandleCopyData(HWND sender, const COPYDATASTRUCT* copy) {
    if (copy && copy->dwData == kSrfCandidateOverlayStatusCopyDataId) {
      return static_cast<LRESULT>(HandleStatusQuery(sender, copy));
    }
    return HandleSnapshot(sender, copy) ? TRUE : FALSE;
  }

  void ApplyPendingSnapshot() {
    applySnapshotPosted_ = false;
    if (!pendingSnapshotPresent_) return;
    SrfCandidateOverlaySnapshot snapshot = std::move(pendingSnapshot_);
    const HWND sender = pendingSenderHwnd_;
    pendingSnapshot_ = {};
    pendingSenderHwnd_ = nullptr;
    pendingSnapshotPresent_ = false;
    if (snapshot.sequence != lastAcceptedSequence_) {
      NotifyStatusChanged(sender);
      return;
    }

    if (!snapshot.visible) {
      const HWND previousSender = activeSenderHwnd_;
      // CanAcceptSequence only admits a hide from the owner holding the latest
      // accepted lease. That owner must also be able to withdraw the older
      // frame which was kept during its asynchronous handoff.
      Hide();
      acceptedOwnerId_ = 0;
      acceptedFocusGeneration_ = 0;
      acceptedTargetHwnd_ = nullptr;
      lastAppliedSequence_ = snapshot.sequence;
      if (previousSender && previousSender != sender) {
        NotifyStatusChanged(previousSender);
      }
      NotifyStatusChanged(sender);
      return;
    }
    if (!TargetWindowMatches(snapshot.targetHwnd, snapshot.targetProcessId) ||
        !TargetIsForeground(snapshot.targetHwnd, snapshot.targetProcessId)) {
      Hide();
      NotifyStatusChanged(sender);
      return;
    }

    if (!clientImagePath_.empty()) snapshot.appPath = clientImagePath_;
    snapshot.fullscreenPlacement =
        IsCandidateOverlayTargetFullscreen(snapshot.targetHwnd);
    RefreshConfiguration();
    const SrfAppOptions* options = FindAppOptions(config_, snapshot.appPath);
    RECT refreshedAnchor = {};
    if (!snapshot.caretAnchor &&
        ResolveCandidateGameOverlayAnchor(snapshot.targetHwnd,
                                          snapshot.fullscreenPlacement, options,
                                          &refreshedAnchor)) {
      snapshot.anchor = refreshedAnchor;
    }

    activeSnapshot_ = snapshot;
    activeOwnerId_ = snapshot.ownerId;
    activeTargetProcessId_ = snapshot.targetProcessId;
    activeTargetHwnd_ = snapshot.targetHwnd;
    activeFocusGeneration_ = snapshot.focusGeneration;
    activeSenderHwnd_ = sender;
    lastAppliedSequence_ = snapshot.sequence;
    ShowSnapshot(activeSnapshot_);
    visible_ = candidateWindow_.IsVisible();
    if (!visible_) Hide();
    NotifyStatusChanged(sender);
  }

  void PrepareResources() {
    if (resourcesPrepared_) return;
    RefreshConfiguration();
    SrfCandidateOverlaySnapshot initial = {};
    const SrfUIStyle style = ResolveOverlayStyle(config_, initial);
    candidateWindow_.SetGameOverlay(true, false, nullptr);
    candidateWindow_.SetStyle(style);
    candidateWindow_.PrepareResources();
    resourcesPrepared_ = true;
  }

  void NotifyStatusChanged(HWND sender) const {
    if (!sender || !IsWindow(sender) ||
        !WindowHasClass(sender, SrfCandidateOverlayClientWindowClass())) {
      return;
    }
    DWORD senderProcessId = 0;
    GetWindowThreadProcessId(sender, &senderProcessId);
    if (senderProcessId != clientProcessId_) return;
    const UINT message = SrfCandidateOverlayStatusChangedMessage();
    if (message != 0) (void)PostMessageW(sender, message, 0, 0);
  }

  void RefreshConfiguration() {
    const std::uint64_t version = GetSrfConfigVersion();
    if (!configLoaded_ || version == 0 || version != configVersion_) {
      config_ = LoadSrfConfig();
      configVersion_ = GetSrfConfigVersion();
      configLoaded_ = true;
    }
  }

  void ShowSnapshot(const SrfCandidateOverlaySnapshot& snapshot) {
    RefreshConfiguration();
    const SrfUIStyle style = ResolveOverlayStyle(config_, snapshot);
    candidateWindow_.SetGameOverlay(true, snapshot.fullscreenPlacement,
                                    snapshot.targetHwnd);
    candidateWindow_.SetStyle(style);
    candidateWindow_.Show(
        snapshot.title, snapshot.items, snapshot.comments, snapshot.labels,
        snapshot.pinnedItems, snapshot.clipboardItems, snapshot.pageIndex,
        snapshot.totalPages, snapshot.selectedInPage, snapshot.anchor,
        snapshot.modeTags, false, snapshot.pendingVisual);
  }

  void Hide() {
    candidateWindow_.Hide();
    visible_ = false;
    activeOwnerId_ = 0;
    activeTargetProcessId_ = 0;
    activeTargetHwnd_ = nullptr;
    activeFocusGeneration_ = 0;
    activeSenderHwnd_ = nullptr;
    activeSnapshot_ = {};
  }

  void OnWatchTimer() {
    const ULONGLONG now = GetTickCount64();
    if (!clientProcess_ || WaitForSingleObject(clientProcess_, 0) != WAIT_TIMEOUT) {
      PostQuitMessage(0);
      return;
    }
    if (visible_) {
      if (!TargetWindowMatches(activeTargetHwnd_, activeTargetProcessId_) ||
          !TargetIsForeground(activeTargetHwnd_, activeTargetProcessId_)) {
        const HWND sender = activeSenderHwnd_;
        Hide();
        NotifyStatusChanged(sender);
      } else if (IsCandidateOverlayTargetFullscreen(activeTargetHwnd_) !=
                 activeSnapshot_.fullscreenPlacement) {
        OnCandidateEnvironmentChanged();
      }
      return;
    }
    if (now - lastMessageTick_ >= kOverlayHiddenIdleExitMs) {
      PostQuitMessage(0);
    }
  }

  static LRESULT CALLBACK WndProc(HWND hwnd, UINT message, WPARAM wParam,
                                  LPARAM lParam) {
    CandidateOverlayHost* self = reinterpret_cast<CandidateOverlayHost*>(
        GetWindowLongPtrW(hwnd, GWLP_USERDATA));
    if (message == WM_NCCREATE) {
      const auto* create = reinterpret_cast<const CREATESTRUCTW*>(lParam);
      self = static_cast<CandidateOverlayHost*>(create->lpCreateParams);
      SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    }
    if (!self) return DefWindowProcW(hwnd, message, wParam, lParam);

    switch (message) {
      case WM_COPYDATA:
        return self->HandleCopyData(
            reinterpret_cast<HWND>(wParam),
            reinterpret_cast<const COPYDATASTRUCT*>(lParam));
      case kOverlayApplySnapshotMessage:
        self->ApplyPendingSnapshot();
        return 0;
      case WM_TIMER:
        if (wParam == kOverlayWatchTimerId) {
          self->OnWatchTimer();
          return 0;
        }
        break;
      case WM_DISPLAYCHANGE:
        self->OnCandidateEnvironmentChanged();
        return 0;
      case WM_SETTINGCHANGE:
        if (wParam == SPI_SETWORKAREA) self->OnCandidateEnvironmentChanged();
        return 0;
      case WM_QUERYENDSESSION:
        return TRUE;
      case WM_CLOSE:
        self->Hide();
        DestroyWindow(hwnd);
        PostQuitMessage(0);
        return 0;
      case WM_ENDSESSION:
        if (wParam) PostQuitMessage(0);
        return 0;
      case WM_DESTROY:
        KillTimer(hwnd, kOverlayWatchTimerId);
        self->candidateWindow_.Destroy();
        return 0;
      default:
        break;
    }
    return DefWindowProcW(hwnd, message, wParam, lParam);
  }

  HINSTANCE instance_ = nullptr;
  HWND controlWindow_ = nullptr;
  HANDLE clientProcess_ = nullptr;
  DWORD clientProcessId_ = 0;
  std::wstring clientImagePath_;
  SrfCandidateOverlayAuthSecret authSecret_ = {};
  CCandidateWindow candidateWindow_;
  SrfConfig config_ = {};
  std::uint64_t configVersion_ = 0;
  bool configLoaded_ = false;
  bool resourcesPrepared_ = false;
  bool visible_ = false;
  std::uint64_t activeOwnerId_ = 0;
  DWORD activeTargetProcessId_ = 0;
  HWND activeTargetHwnd_ = nullptr;
  HWND activeSenderHwnd_ = nullptr;
  std::uint64_t activeFocusGeneration_ = 0;
  std::uint64_t acceptedOwnerId_ = 0;
  std::uint64_t acceptedFocusGeneration_ = 0;
  HWND acceptedTargetHwnd_ = nullptr;
  std::uint64_t lastAcceptedOwnerId_ = 0;
  std::uint64_t lastAcceptedSequence_ = 0;
  std::uint64_t lastAppliedSequence_ = 0;
  ULONGLONG lastMessageTick_ = 0;
  SrfCandidateOverlaySnapshot activeSnapshot_ = {};
  SrfCandidateOverlaySnapshot pendingSnapshot_ = {};
  HWND pendingSenderHwnd_ = nullptr;
  bool pendingSnapshotPresent_ = false;
  bool applySnapshotPosted_ = false;
};

struct OverlayLaunchOptions {
  DWORD clientProcessId = 0;
  std::wstring controlToken;
  HANDLE authHandle = nullptr;
};

bool ParseUnsignedValue(const wchar_t* text, unsigned long long* value) {
  if (!text || !*text || !value || *text == L'-') return false;
  wchar_t* end = nullptr;
  const unsigned long long parsed = _wcstoui64(text, &end, 10);
  if (!end || *end != L'\0') return false;
  *value = parsed;
  return true;
}

bool ValidControlToken(const std::wstring& token) {
  if (token.size() < 8 || token.size() > 96) return false;
  return std::all_of(token.begin(), token.end(), [](wchar_t ch) {
    return std::iswalnum(ch) || ch == L'{' || ch == L'}' || ch == L'-' ||
           ch == L'_';
  });
}

bool ParseLaunchOptions(OverlayLaunchOptions* options) {
  if (!options) return false;
  int count = 0;
  wchar_t** args = CommandLineToArgvW(GetCommandLineW(), &count);
  if (!args) return false;
  bool modeSeen = false;
  bool valid = true;
  for (int index = 1; index < count && valid; ++index) {
    const std::wstring arg = args[index];
    if (arg == L"--candidate-overlay") {
      modeSeen = true;
    } else if (arg == L"--client-pid" && index + 1 < count) {
      unsigned long long parsed = 0;
      valid = ParseUnsignedValue(args[++index], &parsed) && parsed != 0 &&
              parsed <= MAXDWORD;
      if (valid) options->clientProcessId = static_cast<DWORD>(parsed);
    } else if (arg == L"--control-token" && index + 1 < count) {
      options->controlToken = args[++index];
      valid = ValidControlToken(options->controlToken);
    } else if (arg == L"--auth-handle" && index + 1 < count) {
      unsigned long long parsed = 0;
      valid = ParseUnsignedValue(args[++index], &parsed) && parsed != 0 &&
              parsed <= (std::numeric_limits<std::uintptr_t>::max)();
      if (valid) {
        options->authHandle = reinterpret_cast<HANDLE>(
            static_cast<std::uintptr_t>(parsed));
      }
    } else {
      valid = false;
    }
  }
  LocalFree(args);
  return valid && modeSeen && options->clientProcessId != 0 &&
         ValidControlToken(options->controlToken) && options->authHandle &&
         options->authHandle != INVALID_HANDLE_VALUE;
}

bool ReadAuthSecret(HANDLE handle, SrfCandidateOverlayAuthSecret* secret) {
  if (!handle || handle == INVALID_HANDLE_VALUE || !secret) return false;
  DWORD bytesRead = 0;
  const BOOL read = ReadFile(handle, secret, sizeof(*secret), &bytesRead, nullptr);
  CloseHandle(handle);
  return read && bytesRead == sizeof(*secret) && secret->Valid();
}

}  // namespace

HMODULE SrfTip_GetDllModule() { return GetModuleHandleW(nullptr); }

void SrfTsfDiagnosticLog(const wchar_t* tag, const wchar_t* message) {
  std::wstring line = L"[kaixin-overlay] ";
  if (tag) line += tag;
  line += L": ";
  if (message) line += message;
  line += L"\r\n";
  OutputDebugStringW(line.c_str());
}

void SrfTsfPerfLog(const wchar_t* tag, const wchar_t* message) {
  SrfTsfDiagnosticLog(tag, message);
}

extern "C" void SrfTip_BackgroundWorkerAddRef() {}
extern "C" void SrfTip_BackgroundWorkerRelease() {}

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE, PWSTR, int) {
  if (CandidatePreviewRequested()) {
    (void)SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    const HRESULT previewCom = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    const int previewResult = RunCandidatePreview();
    ShutdownCandidateWindowRendering();
    if (SUCCEEDED(previewCom)) CoUninitialize();
    return previewResult;
  }
  OverlayLaunchOptions options = {};
  if (!ParseLaunchOptions(&options)) return 1;
  SrfCandidateOverlayAuthSecret authSecret = {};
  if (!ReadAuthSecret(options.authHandle, &authSecret)) return 1;

  (void)SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
  const HRESULT comResult = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  CandidateOverlayHost host;
  const bool initialized = host.Initialize(instance, options.clientProcessId,
                                           options.controlToken, authSecret);
  const int result = initialized ? host.Run() : 2;
  ShutdownCandidateWindowRendering();
  if (SUCCEEDED(comResult)) CoUninitialize();
  return result;
}
