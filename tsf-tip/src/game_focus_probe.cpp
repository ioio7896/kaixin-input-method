#include "background_worker.h"
#include "game_focus_probe.h"
#include "game_input_policy.h"
#include <UIAutomation.h>
#include <dwmapi.h>
#include <oleacc.h>
#include <atomic>
#include <wrl/client.h>
#include <wrl/implements.h>
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
  std::atomic<std::uint64_t> focusRevision{0};
};
// The worker has an idle deadline; no UI-thread join or provider-owned object
// crosses the apartment boundary. Its module reference prevents DLL unload.
ProbeState& State() {
  static ProbeState state;
  return state;
}
class FocusChangedHandler final
    : public Microsoft::WRL::RuntimeClass<
          Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,
          IUIAutomationFocusChangedEventHandler, Microsoft::WRL::FtmBase> {
 public:
  STDMETHODIMP HandleFocusChangedEvent(IUIAutomationElement*) override {
    State().focusRevision.fetch_add(1, std::memory_order_relaxed);
    return S_OK;
  }
};

bool SameRequest(const SrfGameFocusEvidence& a, const SrfGameFocusEvidence& b) {
  return a.target == b.target && a.nativeFocus == b.nativeFocus && a.processId == b.processId &&
         a.context == b.context && a.generation == b.generation;
}
bool TargetMatches(const SrfGameFocusEvidence& request) {
  DWORD pid = 0;
  HWND foreground = GetForegroundWindow();
  if (!foreground || GetAncestor(foreground, GA_ROOT) != request.target ||
      !IsWindowVisible(request.target))
    return false;
  const DWORD thread = GetWindowThreadProcessId(request.target, &pid);
  GUITHREADINFO info = {sizeof(info)};
  return pid == request.processId && thread && GetGUIThreadInfo(thread, &info) &&
         info.hwndFocus == request.nativeFocus;
}

bool EditablePattern(IUIAutomationElement* element, CONTROLTYPEID type, HWND nativeFocus,
                     bool* needsStability) {
  ComPtr<IUIAutomationValuePattern> value;
  BOOL readOnly = TRUE;
  if (SUCCEEDED(element->GetCurrentPatternAs(UIA_ValuePatternId, IID_PPV_ARGS(&value))) && value &&
      SUCCEEDED(value->get_CurrentIsReadOnly(&readOnly))) {
    if (readOnly) return false;
    wchar_t nativeClass[96] = {};
    GetClassNameW(nativeFocus, nativeClass, _countof(nativeClass));
    if (type == UIA_EditControlTypeId || type == UIA_DocumentControlTypeId ||
        type == UIA_PaneControlTypeId ||
        (type == UIA_CustomControlTypeId &&
         wcscmp(nativeClass, L"Chrome_RenderWidgetHostHWND") == 0)) {
      *needsStability = type != UIA_EditControlTypeId;
      return true;
    }
  }
  // Read the provider's editability attribute without requesting field content.
  ComPtr<IUIAutomationTextPattern> text;
  ComPtr<IUIAutomationTextRange> range;
  *needsStability = true;
  if (SUCCEEDED(element->GetCurrentPatternAs(UIA_TextPatternId, IID_PPV_ARGS(&text))) && text &&
      SUCCEEDED(text->get_DocumentRange(&range)) && range) {
    VARIANT attribute;
    VariantInit(&attribute);
    const bool hasReadOnlyAttribute =
        SUCCEEDED(range->GetAttributeValue(UIA_IsReadOnlyAttributeId, &attribute)) &&
        attribute.vt == VT_BOOL;
    const bool writable = hasReadOnlyAttribute && attribute.boolVal == VARIANT_FALSE;
    VariantClear(&attribute);
    if (hasReadOnlyAttribute) return writable;
  }
  ComPtr<IUIAutomationLegacyIAccessiblePattern> legacy;
  DWORD role = 0, state = 0;
  return SUCCEEDED(
             element->GetCurrentPatternAs(UIA_LegacyIAccessiblePatternId, IID_PPV_ARGS(&legacy))) &&
         legacy && SUCCEEDED(legacy->get_CurrentRole(&role)) && role == ROLE_SYSTEM_TEXT &&
         SUCCEEDED(legacy->get_CurrentState(&state)) && (state & STATE_SYSTEM_FOCUSABLE) &&
         !(state & (STATE_SYSTEM_READONLY | STATE_SYSTEM_PROTECTED | STATE_SYSTEM_UNAVAILABLE));
}

bool HasChatFieldBounds(HWND target, const RECT& bounds) {
  RECT frame = {};
  if (FAILED(DwmGetWindowAttribute(target, DWMWA_EXTENDED_FRAME_BOUNDS, &frame, sizeof(frame))))
    return false;
  return bounds.left >= frame.left && bounds.top >= frame.top && bounds.right <= frame.right &&
         bounds.bottom <= frame.bottom &&
         (static_cast<LONGLONG>(bounds.bottom) - bounds.top) * 2 <
             static_cast<LONGLONG>(frame.bottom) - frame.top;
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
      RECT bounds = {};
      if (SUCCEEDED(focused->get_CurrentProcessId(&pid)) &&
          pid == static_cast<int>(request.processId) &&
          SUCCEEDED(focused->get_CurrentControlType(&type)) &&
          SUCCEEDED(focused->get_CurrentHasKeyboardFocus(&keyboardFocus)) && keyboardFocus &&
          SUCCEEDED(focused->get_CurrentIsEnabled(&enabled)) && enabled &&
          SUCCEEDED(focused->get_CurrentIsPassword(&password)) && !password &&
          SUCCEEDED(focused->get_CurrentIsOffscreen(&offscreen)) && !offscreen &&
          SUCCEEDED(focused->get_CurrentBoundingRectangle(&bounds)) && bounds.right > bounds.left &&
          bounds.bottom > bounds.top &&
          EditablePattern(focused.Get(), type, request.nativeFocus, &result.needsStability) &&
          (!result.needsStability || HasChatFieldBounds(request.target, bounds))) {
        SAFEARRAY* runtime = nullptr;
        if (SUCCEEDED(focused->GetRuntimeId(&runtime)) && runtime) {
          LONG first = 0, last = -1;
          std::uint64_t hash = 14695981039346656037ull;
          VARTYPE arrayType = VT_EMPTY;
          if (SafeArrayGetDim(runtime) == 1 &&
              SUCCEEDED(SafeArrayGetVartype(runtime, &arrayType)) && arrayType == VT_I4 &&
              SUCCEEDED(SafeArrayGetLBound(runtime, 1, &first)) &&
              SUCCEEDED(SafeArrayGetUBound(runtime, 1, &last)) && last >= first &&
              static_cast<LONGLONG>(last) - first < 128) {
            bool valid = true;
            for (LONGLONG offset = 0; offset <= static_cast<LONGLONG>(last) - first; ++offset) {
              LONG index = static_cast<LONG>(static_cast<LONGLONG>(first) + offset);
              int part = 0;
              if (FAILED(SafeArrayGetElement(runtime, &index, &part))) {
                valid = false;
                break;
              }
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
            SUCCEEDED(automation->CompareElements(focused.Get(), stillFocused.Get(), &same)) &&
            same) {
          result.editable = true;
          result.bounds = bounds;
        }
      }
    }
  }
  result.checked = started;
  return result;
}
void Worker() {
  const HRESULT initialized = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
  {
    ComPtr<IUIAutomation> automation;
    if (SUCCEEDED(initialized)) {
      ComPtr<IUIAutomation2> bounded;
      // No unbounded fallback: Windows 10/11 provide the timeout-aware client.
      if (SUCCEEDED(CoCreateInstance(CLSID_CUIAutomation8, nullptr, CLSCTX_INPROC_SERVER,
                                     IID_PPV_ARGS(&bounded))) &&
          SUCCEEDED(bounded->put_ConnectionTimeout(150)) &&
          SUCCEEDED(bounded->put_TransactionTimeout(150)))
        bounded.As(&automation);
    }
    auto focusHandler = Microsoft::WRL::Make<FocusChangedHandler>();
    const bool watchingFocus =
        automation && focusHandler &&
        SUCCEEDED(automation->AddFocusChangedEventHandler(nullptr, focusHandler.Get()));
    ULONGLONG lastProbe = 0;
    std::uint64_t lastFocusRevision = 0;
    SrfGameFocusEvidence lastRequest;
    for (;;) {
      SrfGameFocusEvidence request;
      ULONGLONG requested = 0;
      {
        auto& state = State();
        std::lock_guard<std::mutex> lock(state.mutex);
        requested = state.requested;
        if (GetTickCount64() - requested > 1000) {
          state.running = false;
          break;
        }
        request = state.request;
      }
      const auto focusRevision = State().focusRevision.load(std::memory_order_relaxed);
      if (requested != lastProbe || focusRevision != lastFocusRevision ||
          !SameRequest(request, lastRequest)) {
        auto result = Probe(automation.Get(), request);
        result.focusRevision = focusRevision;
        auto& state = State();
        std::lock_guard<std::mutex> lock(state.mutex);
        if (SameRequest(request, state.request) &&
            focusRevision == state.focusRevision.load(std::memory_order_relaxed)) {
          const auto& previous = state.result;
          const bool sameField = previous.editable && previous.element == result.element &&
                                 previous.focusRevision == focusRevision &&
                                 SameRequest(previous, result) &&
                                 SrfGameFocusEvidenceFresh(previous.checked, result.checked) &&
                                 EqualRect(&previous.bounds, &result.bounds);
          if (result.editable)
            result.stableSince = sameField ? previous.stableSince : result.checked;
          state.result = result;
        }
        lastProbe = requested;
        lastRequest = request;
        lastFocusRevision = focusRevision;
      }
      Sleep(50);
    }
    if (watchingFocus) automation->RemoveFocusChangedEventHandler(focusHandler.Get());
  }
  if (SUCCEEDED(initialized)) CoUninitialize();
}
}  // namespace

bool SrfPollGameFocusEvidence(const SrfGameFocusEvidence& request, SrfGameFocusEvidence* result) {
  auto& state = State();
  std::lock_guard<std::mutex> lock(state.mutex);
  const ULONGLONG now = GetTickCount64();
  const bool matches = SameRequest(state.request, request);
  if (!matches || now - state.requested >= 50) {
    state.request = request;
    state.requested = now;
  }
  if (!state.running) {
    state.running = true;
    if (!StartDetachedBackgroundWorker(Worker)) state.running = false;
  }
  if (!result || !SameRequest(state.result, request) ||
      state.result.focusRevision != state.focusRevision.load(std::memory_order_relaxed) ||
      !SrfGameFocusEvidenceFresh(state.result.checked, now))
    return false;
  *result = state.result;
  return true;
}
