#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <cstdint>
#include <string>

#include "candidate_overlay_client.h"

HMODULE SrfTip_GetDllModule() { return GetModuleHandleW(nullptr); }
void SrfTsfDiagnosticLog(const wchar_t* tag, const wchar_t* message) {
  wchar_t line[1024] = {};
  _snwprintf_s(line, _TRUNCATE, L"[overlay-smoke] %ls: %ls\r\n",
               tag ? tag : L"", message ? message : L"");
  OutputDebugStringW(line);

  const HANDLE errorHandle = GetStdHandle(STD_ERROR_HANDLE);
  if (!errorHandle || errorHandle == INVALID_HANDLE_VALUE) return;
  char utf8[4096] = {};
  const int bytes = WideCharToMultiByte(CP_UTF8, 0, line, -1, utf8,
                                        static_cast<int>(sizeof(utf8)),
                                        nullptr, nullptr);
  if (bytes <= 1) return;
  DWORD written = 0;
  (void)WriteFile(errorHandle, utf8, static_cast<DWORD>(bytes - 1), &written,
                  nullptr);
}

namespace {

constexpr wchar_t kSmokeTargetClass[] = L"SRF_IME_Overlay_Smoke_Target";
constexpr wchar_t kCandidateWindowClass[] = L"SRF_TSF_Candidate_Window";
bool g_statusChanged = false;

LRESULT CALLBACK SmokeTargetProc(HWND hwnd, UINT message, WPARAM wParam, LPARAM lParam) {
  if (message == WM_DESTROY) return 0;
  return DefWindowProcW(hwnd, message, wParam, lParam);
}

LRESULT CALLBACK FakeControlProc(HWND hwnd, UINT message, WPARAM wParam,
                                LPARAM lParam) {
  return DefWindowProcW(hwnd, message, wParam, lParam);
}

LRESULT CALLBACK SmokeSenderProc(HWND hwnd, UINT message, WPARAM wParam,
                                LPARAM lParam) {
  if (message == SrfCandidateOverlayStatusChangedMessage()) {
    g_statusChanged = true;
    return 0;
  }
  return DefWindowProcW(hwnd, message, wParam, lParam);
}

bool PumpUntil(DWORD timeoutMs, bool (*predicate)()) {
  const ULONGLONG deadline = GetTickCount64() + timeoutMs;
  do {
    MSG message = {};
    while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
      TranslateMessage(&message);
      DispatchMessageW(&message);
    }
    if (predicate()) return true;
    Sleep(20);
  } while (GetTickCount64() < deadline);
  return predicate();
}

HWND FindVisibleExternalCandidateWindow() {
  HWND window = nullptr;
  while ((window = FindWindowExW(nullptr, window, kCandidateWindowClass, nullptr)) != nullptr) {
    DWORD processId = 0;
    GetWindowThreadProcessId(window, &processId);
    if (processId != 0 && processId != GetCurrentProcessId() && IsWindowVisible(window)) {
      return window;
    }
  }
  return nullptr;
}

bool CandidateVisible() { return FindVisibleExternalCandidateWindow() != nullptr; }
bool CandidateHidden() { return FindVisibleExternalCandidateWindow() == nullptr; }

bool TargetIsForeground(HWND target) {
  if (!target || !IsWindow(target)) return false;
  const HWND foreground = GetForegroundWindow();
  if (!foreground) return false;
  const HWND foregroundRoot = GetAncestor(foreground, GA_ROOT);
  const HWND targetRoot = GetAncestor(target, GA_ROOT);
  return (foregroundRoot ? foregroundRoot : foreground) ==
         (targetRoot ? targetRoot : target);
}

void TryMakeForeground(HWND target) {
  if (!target) return;
  const HWND foreground = GetForegroundWindow();
  const DWORD currentThread = GetCurrentThreadId();
  DWORD foregroundThread = 0;
  if (foreground) {
    foregroundThread = GetWindowThreadProcessId(foreground, nullptr);
  }
  const bool attached = foregroundThread != 0 && foregroundThread != currentThread &&
                        AttachThreadInput(currentThread, foregroundThread, TRUE);
  BringWindowToTop(target);
  SetActiveWindow(target);
  SetFocus(target);
  SetForegroundWindow(target);
  if (attached) {
    AttachThreadInput(currentThread, foregroundThread, FALSE);
  }
}

bool TryAcquireForeground(HWND target, DWORD timeoutMs) {
  const ULONGLONG deadline = GetTickCount64() + timeoutMs;
  do {
    TryMakeForeground(target);
    MSG message = {};
    while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
      TranslateMessage(&message);
      DispatchMessageW(&message);
    }
    if (TargetIsForeground(target)) return true;
    Sleep(20);
  } while (GetTickCount64() < deadline);
  TryMakeForeground(target);
  return TargetIsForeground(target);
}

}  // namespace

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE, PWSTR, int) {
  WNDCLASSEXW windowClass = {};
  windowClass.cbSize = sizeof(windowClass);
  windowClass.hInstance = instance;
  windowClass.lpfnWndProc = SmokeTargetProc;
  windowClass.lpszClassName = kSmokeTargetClass;
  if (!RegisterClassExW(&windowClass) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) return 1;

  WNDCLASSEXW fakeControlClass = {};
  fakeControlClass.cbSize = sizeof(fakeControlClass);
  fakeControlClass.hInstance = instance;
  fakeControlClass.lpfnWndProc = FakeControlProc;
  fakeControlClass.lpszClassName = SrfCandidateOverlayControlWindowClass();
  if (!RegisterClassExW(&fakeControlClass) &&
      GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
    return 1;
  }
  HWND fakeControl = CreateWindowExW(
      0, SrfCandidateOverlayControlWindowClass(), L"class-squatter", 0, 0, 0,
      0, 0, HWND_MESSAGE, nullptr, instance, nullptr);
  if (!fakeControl) return 1;

  WNDCLASSEXW senderClass = {};
  senderClass.cbSize = sizeof(senderClass);
  senderClass.hInstance = instance;
  senderClass.lpfnWndProc = SmokeSenderProc;
  senderClass.lpszClassName = SrfCandidateOverlayClientWindowClass();
  if (!RegisterClassExW(&senderClass) &&
      GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
    return 1;
  }
  HWND sender = CreateWindowExW(
      0, SrfCandidateOverlayClientWindowClass(), L"", 0, 0, 0, 0, 0,
      HWND_MESSAGE, nullptr, instance, nullptr);
  if (!sender) return 1;

  HWND target = CreateWindowExW(0, kSmokeTargetClass, L"Kaixin overlay smoke", WS_OVERLAPPEDWINDOW,
                                100, 100, 800, 500, nullptr, nullptr, instance, nullptr);
  if (!target) return 2;
  ShowWindow(target, SW_SHOW);
  if (!TryAcquireForeground(target, 1000)) {
    // A service-session CI worker has no foreground desktop to validate.
    SrfTsfDiagnosticLog(L"test", L"skipped: could not acquire foreground");
    DestroyWindow(fakeControl);
    DestroyWindow(target);
    return 0;
  }
  const HWND overlayTarget = target;
  const DWORD overlayTargetProcessId = GetCurrentProcessId();
  RECT targetRect = {};
  GetWindowRect(overlayTarget, &targetRect);

  wchar_t modulePath[32768] = {};
  const DWORD moduleLength = GetModuleFileNameW(nullptr, modulePath, 32768);
  SrfCandidateOverlaySnapshot snapshot = {};
  snapshot.visible = true;
  snapshot.gameCompact = true;
  snapshot.layoutResolved = true;
  snapshot.horizontalLayout = true;
  snapshot.horizontalCompact = true;
  snapshot.anchorPhysical = true;
  snapshot.targetProcessId = overlayTargetProcessId;
  snapshot.targetHwnd = overlayTarget;
  snapshot.focusGeneration = 1;
  snapshot.anchor = {targetRect.left + 50, targetRect.bottom - 80,
                     targetRect.left + 51, targetRect.bottom - 60};
  snapshot.pageIndex = 1;
  snapshot.totalPages = 1;
  snapshot.selectedInPage = 0;
  snapshot.appPath = moduleLength ? std::wstring(modulePath, moduleLength) : L"overlay-smoke.exe";
  snapshot.title = L"nihao";
  snapshot.items = {L"你好", L"你号", L"拟好"};
  snapshot.comments = {L"", L"", L""};
  snapshot.labels = {L"1", L"2", L"3"};
  snapshot.pinnedItems = {false, false, false};
  snapshot.clipboardItems = {false, false, false};
  snapshot.modeTags = {L"游戏"};

  auto& client = GetExternalCandidateOverlayClient();
  constexpr std::uint64_t kOwnerId = 1;
  SrfTsfDiagnosticLog(L"test", L"running external overlay checks");
  client.Prewarm();
  bool accepted = false;
  std::uint64_t acceptedSequence = 0;
  const ULONGLONG deadline = GetTickCount64() + 3000;
  do {
    // Helper cold-start work or another desktop window can briefly take the
    // foreground after the initial check. The production helper deliberately
    // rejects snapshots for anything except the exact foreground root, so
    // restore that invariant on every retry.
    if (!TargetIsForeground(overlayTarget)) {
      (void)TryAcquireForeground(overlayTarget, 100);
    }
    accepted = client.Show(sender, kOwnerId, snapshot, &acceptedSequence);
    if (accepted) break;
    Sleep(30);
  } while (GetTickCount64() < deadline);
  if (!accepted || acceptedSequence == 0 || !PumpUntil(1000, CandidateVisible)) {
    std::wstring detail = L"show failed accepted=";
    detail += accepted ? L"1" : L"0";
    detail += L" sequence=";
    detail += std::to_wstring(acceptedSequence);
    detail += L" visible=";
    detail += CandidateVisible() ? L"1" : L"0";
    const HWND foreground = GetForegroundWindow();
    DWORD foregroundProcessId = 0;
    if (foreground) {
      GetWindowThreadProcessId(foreground, &foregroundProcessId);
    }
    detail += L" foregroundHwnd=";
    detail += std::to_wstring(static_cast<unsigned long long>(
        reinterpret_cast<std::uintptr_t>(foreground)));
    detail += L" foregroundPid=";
    detail += std::to_wstring(foregroundProcessId);
    SrfTsfDiagnosticLog(L"test", detail.c_str());
    DestroyWindow(target);
    return 3;
  }
  const ULONGLONG statusDeadline = GetTickCount64() + 1000;
  SrfCandidateOverlayStatus status = SrfCandidateOverlayStatus::Unavailable;
  do {
    MSG message = {};
    while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
      TranslateMessage(&message);
      DispatchMessageW(&message);
    }
    status = client.QueryStatus(sender, kOwnerId, acceptedSequence);
    // The helper posts the status-changed notification asynchronously.  A
    // synchronous status query can observe SequenceApplied before that posted
    // message reaches this thread's queue, so wait for both observations.
    if (status == SrfCandidateOverlayStatus::SequenceApplied &&
        g_statusChanged) {
      break;
    }
    Sleep(20);
  } while (GetTickCount64() < statusDeadline);
  if (!g_statusChanged || status != SrfCandidateOverlayStatus::SequenceApplied ||
      client.QueryStatus(sender, kOwnerId, acceptedSequence + 1) !=
          SrfCandidateOverlayStatus::OwnerVisible ||
      client.QueryStatus(sender, kOwnerId + 1, acceptedSequence) !=
          SrfCandidateOverlayStatus::Unavailable) {
    std::wstring detail = L"status failed changed=";
    detail += g_statusChanged ? L"1" : L"0";
    detail += L" status=";
    detail += std::to_wstring(static_cast<std::uint32_t>(status));
    SrfTsfDiagnosticLog(L"test", detail.c_str());
    DestroyWindow(target);
    return 8;
  }

  HWND candidate = FindVisibleExternalCandidateWindow();
  if (!candidate || (GetWindowLongPtrW(candidate, GWL_EXSTYLE) & WS_EX_TRANSPARENT) == 0) {
    DestroyWindow(target);
    return 4;
  }
  DWORD_PTR hitResult = 0;
  const LPARAM hitPoint = MAKELPARAM(snapshot.anchor.left, snapshot.anchor.bottom);
  if (!SendMessageTimeoutW(candidate, WM_NCHITTEST, 0, hitPoint,
                           SMTO_ABORTIFHUNG | SMTO_BLOCK, 500, &hitResult) ||
      static_cast<LRESULT>(hitResult) != HTTRANSPARENT) {
    DestroyWindow(target);
    return 5;
  }
  PostMessageW(candidate, WM_DISPLAYCHANGE, 32, MAKELPARAM(1920, 1080));
  if (!PumpUntil(1500, CandidateVisible)) {
    DestroyWindow(target);
    return 6;
  }

  // Exercise a same-process owner handoff. Once owner 2 is accepted, owner 1
  // must be marked superseded and its old external frame must not survive an
  // immediate owner-2 Hide, even if the owner-2 Show has not applied yet.
  constexpr std::uint64_t kTakeoverOwnerId = 2;
  SrfCandidateOverlaySnapshot takeover = snapshot;
  takeover.focusGeneration = 2;
  takeover.title = L"shijie";
  takeover.items = {L"世界", L"视界", L"市界"};
  std::uint64_t takeoverSequence = 0;
  if (!client.Show(sender, kTakeoverOwnerId, takeover, &takeoverSequence) ||
      takeoverSequence <= acceptedSequence) {
    DestroyWindow(target);
    return 10;
  }
  if (client.QueryStatus(sender, kOwnerId, acceptedSequence) !=
      SrfCandidateOverlayStatus::Superseded) {
    DestroyWindow(target);
    return 11;
  }

  client.Hide(sender, kTakeoverOwnerId, overlayTarget,
              overlayTargetProcessId, 2);
  if (!PumpUntil(1000, CandidateHidden)) {
    DestroyWindow(target);
    return 7;
  }
  if (client.QueryStatus(sender, kOwnerId, acceptedSequence) !=
          SrfCandidateOverlayStatus::Superseded ||
      client.QueryStatus(sender, kTakeoverOwnerId, takeoverSequence) !=
          SrfCandidateOverlayStatus::Unavailable) {
    DestroyWindow(target);
    return 12;
  }
  client.Shutdown();
  DestroyWindow(sender);
  DestroyWindow(fakeControl);
  DestroyWindow(target);
  return 0;
}
