#include "game_focus_probe.h"
#include "game_input_policy.h"
#include <UIAutomation.h>
#include <wrl/client.h>
#include <mutex>
#include <new>

extern "C" void SrfTip_BackgroundWorkerAddRef();
extern "C" void SrfTip_BackgroundWorkerRelease();
using Microsoft::WRL::ComPtr;

namespace {
struct ProbeState {
  std::mutex mutex;
  SrfGameFocusEvidence request, result;
  ULONGLONG requested = 0;
  bool running = false;
};
// The worker has an idle deadline; no UI-thread join or provider-owned object
// crosses the apartment boundary. Its module reference prevents DLL unload.
ProbeState& State() { static ProbeState state; return state; }
bool SameRequest(const SrfGameFocusEvidence& a, const SrfGameFocusEvidence& b) {
  return a.target == b.target && a.nativeFocus == b.nativeFocus &&
         a.processId == b.processId && a.context == b.context && a.generation == b.generation;
}
bool TargetMatches(const SrfGameFocusEvidence& request) {
  DWORD pid = 0;
  HWND foreground = GetForegroundWindow();
  if (!foreground || GetAncestor(foreground, GA_ROOT) != request.target ||
      !IsWindowVisible(request.target)) return false;
  const DWORD thread = GetWindowThreadProcessId(request.target, &pid);
  GUITHREADINFO info = {sizeof(info)};
  return pid == request.processId && thread && GetGUIThreadInfo(thread, &info) &&
         info.hwndFocus == request.nativeFocus;
}
SrfGameFocusEvidence Probe(IUIAutomation* automation, const SrfGameFocusEvidence& request) {
  SrfGameFocusEvidence result = request;
  const ULONGLONG started = GetTickCount64();
  result.editable = false;
  result.element = 0;
  result.bounds = {};
  if (automation && TargetMatches(request)) {
    ComPtr<IUIAutomationElement> focused;
    if (SUCCEEDED(automation->GetFocusedElement(&focused)) && focused) {
      int pid = 0;
      CONTROLTYPEID type = 0;
      BOOL keyboardFocus = FALSE, enabled = FALSE, password = TRUE, offscreen = TRUE;
      ComPtr<IUIAutomationValuePattern> value;
      BOOL readOnly = TRUE;
      RECT bounds = {};
      if (SUCCEEDED(focused->get_CurrentProcessId(&pid)) && pid == static_cast<int>(request.processId) &&
          SUCCEEDED(focused->get_CurrentControlType(&type)) && type == UIA_EditControlTypeId &&
          SUCCEEDED(focused->get_CurrentHasKeyboardFocus(&keyboardFocus)) && keyboardFocus &&
          SUCCEEDED(focused->get_CurrentIsEnabled(&enabled)) && enabled &&
          SUCCEEDED(focused->get_CurrentIsPassword(&password)) && !password &&
          SUCCEEDED(focused->get_CurrentIsOffscreen(&offscreen)) && !offscreen &&
          SUCCEEDED(focused->GetCurrentPatternAs(UIA_ValuePatternId, IID_PPV_ARGS(&value))) && value &&
          SUCCEEDED(value->get_CurrentIsReadOnly(&readOnly)) && !readOnly &&
          SUCCEEDED(focused->get_CurrentBoundingRectangle(&bounds)) &&
          bounds.right > bounds.left && bounds.bottom > bounds.top) {
        SAFEARRAY* runtime = nullptr;
        if (SUCCEEDED(focused->GetRuntimeId(&runtime)) && runtime) {
          LONG first = 0, last = -1;
          std::uint64_t hash = 14695981039346656037ull;
          VARTYPE arrayType = VT_EMPTY;
          if (SafeArrayGetDim(runtime) == 1 && SUCCEEDED(SafeArrayGetVartype(runtime, &arrayType)) && arrayType == VT_I4 &&
              SUCCEEDED(SafeArrayGetLBound(runtime, 1, &first)) &&
              SUCCEEDED(SafeArrayGetUBound(runtime, 1, &last)) && last >= first &&
              static_cast<LONGLONG>(last) - first < 128) {
            bool valid = true;
            for (LONGLONG offset = 0; offset <= static_cast<LONGLONG>(last) - first; ++offset) {
              LONG index = static_cast<LONG>(static_cast<LONGLONG>(first) + offset);
              int part = 0;
              if (FAILED(SafeArrayGetElement(runtime, &index, &part))) { valid = false; break; }
              hash = (hash ^ static_cast<std::uint32_t>(part)) * 1099511628211ull;
            }
            if (valid) result.element = hash ? hash : 1;
          }
          SafeArrayDestroy(runtime);
        }
        ComPtr<IUIAutomationElement> stillFocused;
        BOOL same = FALSE;
        if (result.element && TargetMatches(request) &&
            SUCCEEDED(automation->GetFocusedElement(&stillFocused)) && stillFocused &&
            SUCCEEDED(automation->CompareElements(focused.Get(), stillFocused.Get(), &same)) && same) {
          result.editable = true;
          result.bounds = bounds;
        }
      }
    }
  }
  result.checked = started;
  return result;
}
DWORD WINAPI Worker(void* parameter) {
  HMODULE module = static_cast<HMODULE>(parameter);
  const HRESULT initialized = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
  {
    ComPtr<IUIAutomation> automation;
    if (SUCCEEDED(initialized)) {
      ComPtr<IUIAutomation2> bounded;
      // No unbounded fallback: Windows 10/11 provide the timeout-aware client.
      if (SUCCEEDED(CoCreateInstance(CLSID_CUIAutomation8, nullptr, CLSCTX_INPROC_SERVER,
                                     IID_PPV_ARGS(&bounded))) &&
          SUCCEEDED(bounded->put_ConnectionTimeout(150)) &&
          SUCCEEDED(bounded->put_TransactionTimeout(150))) bounded.As(&automation);
    }
    ULONGLONG lastProbe = 0;
    SrfGameFocusEvidence lastRequest;
    for (;;) {
      SrfGameFocusEvidence request;
      ULONGLONG requested = 0;
      {
        auto& state = State();
        std::lock_guard<std::mutex> lock(state.mutex);
        requested = state.requested;
        if (GetTickCount64() - requested > 1000) { state.running = false; break; }
        request = state.request;
      }
      if (requested != lastProbe || !SameRequest(request, lastRequest)) {
        auto result = Probe(automation.Get(), request);
        auto& state = State();
        std::lock_guard<std::mutex> lock(state.mutex);
        if (SameRequest(request, state.request)) state.result = result;
        lastProbe = requested;
        lastRequest = request;
      }
      Sleep(50);
    }
  }
  if (SUCCEEDED(initialized)) CoUninitialize();
  SrfTip_BackgroundWorkerRelease();
  FreeLibraryAndExitThread(module, 0);
}
}

bool SrfPollGameFocusEvidence(const SrfGameFocusEvidence& request, SrfGameFocusEvidence* result) {
  auto& state = State();
  std::lock_guard<std::mutex> lock(state.mutex);
  const ULONGLONG now = GetTickCount64();
  const bool matches = SameRequest(state.request, request);
  if (!matches || now - state.requested >= 100) {
    state.request = request;
    state.requested = now;
  }
  if (!state.running) {
    HMODULE module = nullptr;
    if (GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                           reinterpret_cast<LPCWSTR>(&Worker), &module)) {
      SrfTip_BackgroundWorkerAddRef();
      state.running = true;
      HANDLE worker = CreateThread(nullptr, 0, Worker, module, 0, nullptr);
      if (worker) CloseHandle(worker);
      else { state.running = false; SrfTip_BackgroundWorkerRelease(); FreeLibrary(module); }
    }
  }
  if (!result || !SameRequest(state.result, request) ||
      !SrfGameFocusEvidenceFresh(state.result.checked, now)) return false;
  *result = state.result;
  return true;
}
