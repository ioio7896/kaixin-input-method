#include "candidate_input_pane.h"
#include <new>

extern "C" void SrfTip_BackgroundWorkerAddRef();
extern "C" void SrfTip_BackgroundWorkerRelease();

namespace {
class Handler final : public IFrameworkInputPaneHandler {
  LONG m_refs = 1;
  HWND m_window;

 public:
  explicit Handler(HWND window) : m_window(window) { SrfTip_BackgroundWorkerAddRef(); }
  ~Handler() { SrfTip_BackgroundWorkerRelease(); }
  STDMETHODIMP QueryInterface(REFIID iid, void** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    if (iid != IID_IUnknown && iid != __uuidof(IFrameworkInputPaneHandler)) return E_NOINTERFACE;
    *result = static_cast<IFrameworkInputPaneHandler*>(this);
    AddRef();
    return S_OK;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_refs); }
  STDMETHODIMP_(ULONG) Release() override {
    ULONG refs = InterlockedDecrement(&m_refs);
    if (!refs) delete this;
    return refs;
  }
  STDMETHODIMP Showing(RECT*, BOOL) override {
    PostMessageW(m_window, SrfCandidateInputPane::kChangedMessage, 0, 0);
    return S_OK;
  }
  STDMETHODIMP Hiding(BOOL) override {
    PostMessageW(m_window, SrfCandidateInputPane::kChangedMessage, 0, 0);
    return S_OK;
  }
};
}  // namespace
void SrfCandidateInputPane::Attach(HWND candidate, HWND owner) {
  if (m_pane && m_owner == owner) return;
  Detach();
  m_owner = owner;
  // Hosts can deny the service or have no COM apartment. In that case normal
  // work-area placement remains available; never initialize host COM ourselves.
  if (FAILED(CoCreateInstance(CLSID_FrameworkInputPane, nullptr, CLSCTX_INPROC_SERVER,
                              __uuidof(IFrameworkInputPane), reinterpret_cast<void**>(&m_pane))))
    return;
  auto* handler = new (std::nothrow) Handler(candidate);
  if (!handler) return;
  m_advised = SUCCEEDED(m_pane->AdviseWithHWND(owner ? owner : candidate, handler, &m_cookie));
  handler->Release();
}
void SrfCandidateInputPane::Detach() {
  if (m_pane) {
    if (m_advised) m_pane->Unadvise(m_cookie);
    m_pane->Release();
    m_pane = nullptr;
  }
  m_advised = false;
  m_cookie = 0;
  m_owner = nullptr;
}
RECT SrfCandidateInputPane::Location(HWND candidate) const {
  RECT rect = {};
  if (!m_pane || FAILED(m_pane->Location(&rect)) || IsRectEmpty(&rect)) return {};
  POINT first = {rect.left, rect.top}, last = {rect.right, rect.bottom};
  PhysicalToLogicalPointForPerMonitorDPI(candidate, &first);
  PhysicalToLogicalPointForPerMonitorDPI(candidate, &last);
  return {first.x, first.y, last.x, last.y};
}
