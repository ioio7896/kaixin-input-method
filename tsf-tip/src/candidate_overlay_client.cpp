#include "candidate_overlay_client.h"

#include <cstring>
#include <cwchar>
#include <filesystem>
#include <mutex>
#include <string>
#include <utility>
#include <vector>

extern HMODULE SrfTip_GetDllModule();
extern void SrfTsfDiagnosticLog(const wchar_t* tag, const wchar_t* msg);

namespace {

constexpr ULONGLONG kOverlayLaunchRetryMs = 3000;
constexpr ULONGLONG kOverlayHelperReadyTimeoutMs = 5000;
constexpr UINT kOverlaySendTimeoutMs = 12;

std::filesystem::path OverlayHelperPath() {
  HMODULE module = SrfTip_GetDllModule();
  if (!module) return {};
  std::wstring path(32768, L'\0');
  const DWORD length =
      GetModuleFileNameW(module, path.data(), static_cast<DWORD>(path.size()));
  if (length == 0 || length >= path.size()) return {};
  path.resize(length);
  return std::filesystem::path(path).parent_path() / L"srf_ime_overlay.exe";
}

std::wstring QuoteCommandLineArgument(const std::wstring& value) {
  std::wstring quoted = L"\"";
  std::size_t slashCount = 0;
  for (wchar_t ch : value) {
    if (ch == L'\\') {
      ++slashCount;
      continue;
    }
    if (ch == L'\"') {
      quoted.append(slashCount * 2 + 1, L'\\');
      quoted.push_back(ch);
      slashCount = 0;
      continue;
    }
    quoted.append(slashCount, L'\\');
    slashCount = 0;
    quoted.push_back(ch);
  }
  quoted.append(slashCount * 2, L'\\');
  quoted.push_back(L'\"');
  return quoted;
}

std::wstring CreateControlWindowToken() {
  GUID value = {};
  wchar_t text[64] = {};
  if (SUCCEEDED(CoCreateGuid(&value)) && StringFromGUID2(value, text, 64) > 0) {
    return text;
  }
  LARGE_INTEGER counter = {};
  QueryPerformanceCounter(&counter);
  swprintf_s(text, L"fallback-%lu-%lu-%llu", GetCurrentProcessId(),
             GetCurrentThreadId(),
             static_cast<unsigned long long>(counter.QuadPart));
  return text;
}

bool CreateAuthSecret(SrfCandidateOverlayAuthSecret* secret) {
  if (!secret) return false;
  GUID value = {};
  if (FAILED(CoCreateGuid(&value))) return false;
  static_assert(sizeof(value) == sizeof(*secret), "GUID auth secret size changed");
  std::memcpy(secret, &value, sizeof(value));
  return secret->Valid();
}

bool WindowBelongsToProcess(HWND hwnd, DWORD processId) {
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

}  // namespace

CExternalCandidateOverlayClient& GetExternalCandidateOverlayClient() {
  static CExternalCandidateOverlayClient client;
  return client;
}

CExternalCandidateOverlayClient::~CExternalCandidateOverlayClient() { Shutdown(); }

bool CExternalCandidateOverlayClient::IsHelperProcessAliveLocked() const {
  return helperProcess_ && helperProcessId_ != 0 &&
         WaitForSingleObject(helperProcess_, 0) == WAIT_TIMEOUT;
}

bool CExternalCandidateOverlayClient::IsExpectedHelperWindowLocked(HWND hwnd) const {
  return IsHelperProcessAliveLocked() &&
         WindowBelongsToProcess(hwnd, helperProcessId_) &&
         WindowHasClass(hwnd, SrfCandidateOverlayControlWindowClass());
}

void CExternalCandidateOverlayClient::ResetHelperLocked(bool requestClose) {
  if (requestClose && IsExpectedHelperWindowLocked(helperWindow_)) {
    (void)PostMessageW(helperWindow_, WM_CLOSE, 0, 0);
  }
  helperWindow_ = nullptr;
  helperProcessId_ = 0;
  controlWindowToken_.clear();
  authSecret_ = {};
  activeOwnerId_ = 0;
  helperLaunchTick_ = 0;
  if (helperProcess_) {
    CloseHandle(helperProcess_);
    helperProcess_ = nullptr;
  }
}

void CExternalCandidateOverlayClient::TerminateHelperLocked() {
  if (IsHelperProcessAliveLocked()) {
    (void)TerminateProcess(helperProcess_, ERROR_TIMEOUT);
  }
  ResetHelperLocked(false);
}

HWND CExternalCandidateOverlayClient::FindHelperWindowLocked() {
  if (IsExpectedHelperWindowLocked(helperWindow_)) return helperWindow_;
  helperWindow_ = nullptr;
  if (!IsHelperProcessAliveLocked() || controlWindowToken_.empty()) return nullptr;

  HWND after = nullptr;
  while ((after = FindWindowExW(HWND_MESSAGE, after,
                                SrfCandidateOverlayControlWindowClass(),
                                controlWindowToken_.c_str())) != nullptr) {
    if (IsExpectedHelperWindowLocked(after)) {
      helperWindow_ = after;
      helperLaunchTick_ = 0;
      break;
    }
  }
  return helperWindow_;
}

bool CExternalCandidateOverlayClient::StartHelperIfNeededLocked() {
  if (FindHelperWindowLocked()) return true;
  if (helperProcess_) {
    if (IsHelperProcessAliveLocked()) {
      const ULONGLONG now = GetTickCount64();
      if (helperLaunchTick_ == 0) {
        helperLaunchTick_ = now;
        return false;
      }
      if (now - helperLaunchTick_ < kOverlayHelperReadyTimeoutMs) {
        return false;
      }
      SrfTsfDiagnosticLog(L"candidate-overlay.launch",
                          L"helper control window readiness timed out");
      TerminateHelperLocked();
      lastLaunchAttemptTick_ = 0;
    } else {
      ResetHelperLocked(false);
    }
  }

  const ULONGLONG now = GetTickCount64();
  if (lastLaunchAttemptTick_ != 0 && now - lastLaunchAttemptTick_ < kOverlayLaunchRetryMs) {
    return false;
  }
  lastLaunchAttemptTick_ = now;

  const std::filesystem::path helperPath = OverlayHelperPath();
  std::error_code ec;
  if (helperPath.empty() || !std::filesystem::is_regular_file(helperPath, ec)) {
    SrfTsfDiagnosticLog(L"candidate-overlay.launch", L"helper executable missing");
    return false;
  }

  const std::wstring token = CreateControlWindowToken();
  SrfCandidateOverlayAuthSecret secret = {};
  if (!CreateAuthSecret(&secret)) {
    SrfTsfDiagnosticLog(L"candidate-overlay.launch", L"auth secret generation failed");
    return false;
  }

  SECURITY_ATTRIBUTES pipeSecurity = {};
  pipeSecurity.nLength = sizeof(pipeSecurity);
  pipeSecurity.bInheritHandle = TRUE;
  HANDLE authRead = nullptr;
  HANDLE authWrite = nullptr;
  if (!CreatePipe(&authRead, &authWrite, &pipeSecurity, sizeof(secret)) ||
      !SetHandleInformation(authWrite, HANDLE_FLAG_INHERIT, 0)) {
    const DWORD error = GetLastError();
    if (authRead) CloseHandle(authRead);
    if (authWrite) CloseHandle(authWrite);
    std::wstring line = L"auth pipe creation failed error=";
    line += std::to_wstring(error);
    SrfTsfDiagnosticLog(L"candidate-overlay.launch", line.c_str());
    return false;
  }

  std::wstring commandLine = QuoteCommandLineArgument(helperPath.wstring());
  commandLine += L" --candidate-overlay --client-pid ";
  commandLine += std::to_wstring(GetCurrentProcessId());
  commandLine += L" --control-token ";
  commandLine += QuoteCommandLineArgument(token);
  commandLine += L" --auth-handle ";
  commandLine += std::to_wstring(
      static_cast<unsigned long long>(reinterpret_cast<std::uintptr_t>(authRead)));

  SIZE_T attributeBytes = 0;
  (void)InitializeProcThreadAttributeList(nullptr, 1, 0, &attributeBytes);
  std::vector<std::uint8_t> attributeStorage(attributeBytes);
  auto* attributeList = reinterpret_cast<PPROC_THREAD_ATTRIBUTE_LIST>(
      attributeStorage.data());
  STARTUPINFOEXW startup = {};
  startup.StartupInfo.cb = sizeof(startup);
  startup.lpAttributeList = attributeList;
  PROCESS_INFORMATION process = {};
  HANDLE inheritedHandles[] = {authRead};
  BOOL created = FALSE;
  const bool attributeInitialized =
      attributeBytes != 0 &&
      InitializeProcThreadAttributeList(attributeList, 1, 0, &attributeBytes) != FALSE;
  if (attributeInitialized &&
      UpdateProcThreadAttribute(attributeList, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                                inheritedHandles, sizeof(inheritedHandles), nullptr,
                                nullptr)) {
    created = CreateProcessW(
        helperPath.c_str(), commandLine.data(), nullptr, nullptr, TRUE,
        CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW |
            EXTENDED_STARTUPINFO_PRESENT,
        nullptr, helperPath.parent_path().c_str(), &startup.StartupInfo, &process);
  }
  const DWORD createError = created ? ERROR_SUCCESS : GetLastError();
  if (attributeInitialized) {
    DeleteProcThreadAttributeList(attributeList);
  }
  CloseHandle(authRead);
  authRead = nullptr;
  if (!created) {
    std::wstring line = L"CreateProcess failed error=";
    line += std::to_wstring(createError);
    SrfTsfDiagnosticLog(L"candidate-overlay.launch", line.c_str());
    CloseHandle(authWrite);
    return false;
  }

  DWORD authBytesWritten = 0;
  const BOOL authWritten =
      WriteFile(authWrite, &secret, sizeof(secret), &authBytesWritten, nullptr);
  CloseHandle(authWrite);
  authWrite = nullptr;
  if (!authWritten || authBytesWritten != sizeof(secret)) {
    (void)TerminateProcess(process.hProcess, ERROR_ACCESS_DENIED);
    CloseHandle(process.hThread);
    CloseHandle(process.hProcess);
    SrfTsfDiagnosticLog(L"candidate-overlay.launch", L"auth secret transfer failed");
    return false;
  }

  CloseHandle(process.hThread);
  helperProcess_ = process.hProcess;
  helperProcessId_ = process.dwProcessId;
  controlWindowToken_ = token;
  authSecret_ = secret;
  helperWindow_ = nullptr;
  helperLaunchTick_ = GetTickCount64();
  SrfTsfDiagnosticLog(L"candidate-overlay.launch", L"helper start requested");
  // Do not wait here: a local-window frame is preferable to blocking a game
  // input thread while Windows initializes the helper.
  return false;
}

void CExternalCandidateOverlayClient::Prewarm() {
  std::lock_guard<std::mutex> guard(mutex_);
  (void)StartHelperIfNeededLocked();
}

bool CExternalCandidateOverlayClient::SendLocked(
    HWND senderHwnd, SrfCandidateOverlaySnapshot snapshot,
    std::uint64_t* acceptedSequence) {
  const DWORD currentProcessId = GetCurrentProcessId();
  if (!WindowBelongsToProcess(senderHwnd, currentProcessId) ||
      !WindowHasClass(senderHwnd, SrfCandidateOverlayClientWindowClass()) ||
      snapshot.ownerId == 0 || snapshot.targetProcessId != currentProcessId ||
      (snapshot.targetHwnd &&
       !WindowBelongsToProcess(snapshot.targetHwnd, currentProcessId))) {
    SrfTsfDiagnosticLog(L"candidate-overlay.send", L"snapshot process binding rejected");
    return false;
  }
  HWND helper = FindHelperWindowLocked();
  if (!helper) {
    (void)StartHelperIfNeededLocked();
    return false;
  }
  if (!authSecret_.Valid()) {
    SrfTsfDiagnosticLog(L"candidate-overlay.send", L"helper auth state unavailable");
    TerminateHelperLocked();
    return false;
  }

  snapshot.sourceProcessId = currentProcessId;
  snapshot.authSecret = authSecret_;
  snapshot.sequence = ++nextSequence_;
  std::vector<std::uint8_t> payload;
  if (!SerializeCandidateOverlaySnapshot(snapshot, &payload)) {
    SrfTsfDiagnosticLog(L"candidate-overlay.send", L"snapshot serialization rejected");
    return false;
  }

  COPYDATASTRUCT copy = {};
  copy.dwData = kSrfCandidateOverlayCopyDataId;
  copy.cbData = static_cast<DWORD>(payload.size());
  copy.lpData = payload.data();
  DWORD_PTR accepted = 0;
  SetLastError(ERROR_SUCCESS);
  const LRESULT sent = SendMessageTimeoutW(
      helper, WM_COPYDATA, reinterpret_cast<WPARAM>(senderHwnd),
      reinterpret_cast<LPARAM>(&copy),
      SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
      kOverlaySendTimeoutMs, &accepted);
  if (sent == 0) {
    const DWORD error = GetLastError();
    // A timed-out synchronous message is not guaranteed to be cancelled.
    // This helper belongs only to the current TIP process, so terminate the
    // stateless UI process before falling back to the in-process candidate.
    TerminateHelperLocked();
    std::wstring line = L"snapshot send failed or timed out error=";
    line += std::to_wstring(error);
    SrfTsfDiagnosticLog(L"candidate-overlay.send", line.c_str());
    return false;
  }
  if (accepted == 0) {
    // A rejected update or hide for the currently displayed owner can leave a
    // stale overlay behind. This helper is dedicated and stateless, so tear it
    // down before the caller falls back locally.
    if (snapshot.ownerId == activeOwnerId_) {
      TerminateHelperLocked();
    }
    std::wstring line = L"snapshot rejected error=";
    line += std::to_wstring(GetLastError());
    SrfTsfDiagnosticLog(L"candidate-overlay.send", line.c_str());
    return false;
  }
  if (acceptedSequence) *acceptedSequence = snapshot.sequence;
  return true;
}

bool CExternalCandidateOverlayClient::Show(
    HWND senderHwnd, std::uint64_t ownerId,
    const SrfCandidateOverlaySnapshot& snapshot,
    std::uint64_t* acceptedSequence) {
  std::lock_guard<std::mutex> guard(mutex_);
  if (ownerId == 0) return false;
  SrfCandidateOverlaySnapshot visible = snapshot;
  visible.visible = true;
  visible.ownerId = ownerId;
  const bool shown =
      SendLocked(senderHwnd, std::move(visible), acceptedSequence);
  if (shown) {
    activeOwnerId_ = ownerId;
  }
  return shown;
}

SrfCandidateOverlayStatus CExternalCandidateOverlayClient::QueryStatus(
    HWND senderHwnd, std::uint64_t ownerId, std::uint64_t sequence) {
  std::lock_guard<std::mutex> guard(mutex_);
  const DWORD currentProcessId = GetCurrentProcessId();
  if (ownerId == 0 || sequence == 0 ||
      !WindowBelongsToProcess(senderHwnd, currentProcessId) ||
      !WindowHasClass(senderHwnd, SrfCandidateOverlayClientWindowClass())) {
    return SrfCandidateOverlayStatus::Unavailable;
  }
  HWND helper = FindHelperWindowLocked();
  if (!helper) {
    (void)StartHelperIfNeededLocked();
    return SrfCandidateOverlayStatus::Unavailable;
  }
  if (!authSecret_.Valid()) {
    TerminateHelperLocked();
    return SrfCandidateOverlayStatus::Unavailable;
  }

  SrfCandidateOverlayStatusQuery query = {};
  query.sourceProcessId = currentProcessId;
  query.ownerId = ownerId;
  query.sequence = sequence;
  query.authSecretHigh = authSecret_.high;
  query.authSecretLow = authSecret_.low;
  COPYDATASTRUCT copy = {};
  copy.dwData = kSrfCandidateOverlayStatusCopyDataId;
  copy.cbData = sizeof(query);
  copy.lpData = &query;
  DWORD_PTR result = 0;
  SetLastError(ERROR_SUCCESS);
  const LRESULT sent = SendMessageTimeoutW(
      helper, WM_COPYDATA, reinterpret_cast<WPARAM>(senderHwnd),
      reinterpret_cast<LPARAM>(&copy),
      SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
      kOverlaySendTimeoutMs, &result);
  if (sent == 0) {
    const DWORD error = GetLastError();
    TerminateHelperLocked();
    std::wstring line = L"status query failed or timed out error=";
    line += std::to_wstring(error);
    SrfTsfDiagnosticLog(L"candidate-overlay.status", line.c_str());
    return SrfCandidateOverlayStatus::Unavailable;
  }
  if (result >
      static_cast<DWORD_PTR>(SrfCandidateOverlayStatus::Superseded)) {
    TerminateHelperLocked();
    SrfTsfDiagnosticLog(L"candidate-overlay.status",
                        L"helper returned an invalid status");
    return SrfCandidateOverlayStatus::Unavailable;
  }
  return static_cast<SrfCandidateOverlayStatus>(result);
}

void CExternalCandidateOverlayClient::Hide(HWND senderHwnd,
                                           std::uint64_t ownerId,
                                           HWND targetHwnd, DWORD targetProcessId,
                                           std::uint64_t focusGeneration) {
  std::lock_guard<std::mutex> guard(mutex_);
  if (ownerId == 0 || ownerId != activeOwnerId_) return;
  if (!FindHelperWindowLocked()) {
    activeOwnerId_ = 0;
    return;
  }
  SrfCandidateOverlaySnapshot hidden = {};
  hidden.visible = false;
  hidden.ownerId = ownerId;
  hidden.targetHwnd = targetHwnd;
  hidden.targetProcessId = targetProcessId;
  hidden.focusGeneration = focusGeneration;
  if (SendLocked(senderHwnd, std::move(hidden))) activeOwnerId_ = 0;
}

void CExternalCandidateOverlayClient::Shutdown() {
  std::lock_guard<std::mutex> guard(mutex_);
  TerminateHelperLocked();
}
