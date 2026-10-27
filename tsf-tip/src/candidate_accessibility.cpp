#include "candidate_accessibility.h"
#include <UIAutomation.h>
#include <algorithm>
#include <mutex>
#include <new>
#include <utility>

extern "C" void SrfTip_BackgroundWorkerAddRef();
extern "C" void SrfTip_BackgroundWorkerRelease();

struct SrfCandidateAccessibleState {
  std::mutex mutex;
  HWND hwnd = nullptr;
  bool visible = false;
  bool interactive = false;
  ULONG generation = 1;
  UINT page = 0;
  UINT selection = 0;
  std::wstring title;
  std::vector<std::wstring> items, comments;
  RECT bounds = {};
  std::vector<RECT> itemBounds;
};

namespace {
class Provider final : public IRawElementProviderSimple,
                       public IRawElementProviderFragment,
                       public IRawElementProviderFragmentRoot,
                       public ISelectionProvider,
                       public ISelectionItemProvider,
                       public IInvokeProvider {
  LONG refs_ = 1;
  std::shared_ptr<SrfCandidateAccessibleState> state_;
  int index_;
  ULONG generation_;
  bool Valid() const {
    return state_->hwnd && (index_ < 0 || (state_->visible && generation_ == state_->generation &&
                                           static_cast<size_t>(index_) < state_->items.size()));
  }
  HRESULT Fragment(int index, IRawElementProviderFragment** result) {
    auto* provider = new (std::nothrow) Provider(state_, index, state_->generation);
    if (!provider) return E_OUTOFMEMORY;
    *result = static_cast<IRawElementProviderFragment*>(provider);
    return S_OK;
  }
  HRESULT Action(UINT message) {
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (!Valid()) return UIA_E_ELEMENTNOTAVAILABLE;
    if (index_ < 0 || !state_->visible || !state_->interactive) return UIA_E_ELEMENTNOTENABLED;
    return PostMessageW(state_->hwnd, message, static_cast<WPARAM>(index_), generation_)
               ? S_OK
               : HRESULT_FROM_WIN32(GetLastError());
  }

 public:
  Provider(std::shared_ptr<SrfCandidateAccessibleState> state, int index, ULONG generation)
      : state_(std::move(state)), index_(index), generation_(generation) {
    SrfTip_BackgroundWorkerAddRef();
  }
  ~Provider() { SrfTip_BackgroundWorkerRelease(); }
  STDMETHODIMP QueryInterface(REFIID iid, void** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    if (iid == IID_IUnknown || iid == __uuidof(IRawElementProviderSimple))
      *result = static_cast<IRawElementProviderSimple*>(this);
    else if (iid == __uuidof(IRawElementProviderFragment))
      *result = static_cast<IRawElementProviderFragment*>(this);
    else if (index_ < 0 && iid == __uuidof(IRawElementProviderFragmentRoot))
      *result = static_cast<IRawElementProviderFragmentRoot*>(this);
    else if (index_ < 0 && iid == __uuidof(ISelectionProvider))
      *result = static_cast<ISelectionProvider*>(this);
    else if (index_ >= 0 && iid == __uuidof(ISelectionItemProvider))
      *result = static_cast<ISelectionItemProvider*>(this);
    else if (index_ >= 0 && iid == __uuidof(IInvokeProvider))
      *result = static_cast<IInvokeProvider*>(this);
    else
      return E_NOINTERFACE;
    AddRef();
    return S_OK;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&refs_); }
  STDMETHODIMP_(ULONG) Release() override {
    ULONG count = InterlockedDecrement(&refs_);
    if (!count) delete this;
    return count;
  }
  STDMETHODIMP get_ProviderOptions(ProviderOptions* result) override {
    if (!result) return E_POINTER;
    *result = static_cast<ProviderOptions>(ProviderOptions_ServerSideProvider |
                                           ProviderOptions_UseComThreading);
    return S_OK;
  }
  STDMETHODIMP GetPatternProvider(PATTERNID id, IUnknown** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    if (index_ < 0 && id == UIA_SelectionPatternId)
      return QueryInterface(__uuidof(ISelectionProvider), reinterpret_cast<void**>(result));
    if (index_ >= 0 && id == UIA_SelectionItemPatternId)
      return QueryInterface(__uuidof(ISelectionItemProvider), reinterpret_cast<void**>(result));
    if (index_ >= 0 && id == UIA_InvokePatternId)
      return QueryInterface(__uuidof(IInvokeProvider), reinterpret_cast<void**>(result));
    return S_OK;
  }
  STDMETHODIMP GetPropertyValue(PROPERTYID id, VARIANT* result) override {
    if (!result) return E_POINTER;
    VariantInit(result);
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (!Valid()) return UIA_E_ELEMENTNOTAVAILABLE;
    const wchar_t* text = nullptr;
    switch (id) {
      case UIA_NamePropertyId:
        text = index_ < 0 ? L"开心输入法候选" : state_->items[index_].c_str();
        break;
      case UIA_AutomationIdPropertyId:
        text = index_ < 0 ? L"IME_Candidate_Window" : nullptr;
        break;
      case UIA_HelpTextPropertyId:
        if (index_ >= 0 && static_cast<size_t>(index_) < state_->comments.size())
          text = state_->comments[index_].c_str();
        break;
      case UIA_ControlTypePropertyId:
        result->vt = VT_I4;
        result->lVal = index_ < 0 ? UIA_ListControlTypeId : UIA_ListItemControlTypeId;
        return S_OK;
      case UIA_IsControlElementPropertyId:
      case UIA_IsContentElementPropertyId:
      case UIA_IsEnabledPropertyId:
      case UIA_IsOffscreenPropertyId:
      case UIA_IsKeyboardFocusablePropertyId:
      case UIA_HasKeyboardFocusPropertyId:
      case UIA_SelectionItemIsSelectedPropertyId:
        result->vt = VT_BOOL;
        result->boolVal =
            (id == UIA_IsControlElementPropertyId || id == UIA_IsContentElementPropertyId ||
             (id == UIA_IsEnabledPropertyId && state_->interactive) ||
             (id == UIA_IsOffscreenPropertyId && !state_->visible) ||
             (id == UIA_SelectionItemIsSelectedPropertyId && index_ >= 0 &&
              static_cast<UINT>(index_) == state_->selection))
                ? VARIANT_TRUE
                : VARIANT_FALSE;
        return S_OK;
      default:
        break;
    }
    if (text) {
      result->vt = VT_BSTR;
      result->bstrVal = SysAllocString(text);
      if (!result->bstrVal) return E_OUTOFMEMORY;
    }
    return S_OK;
  }
  STDMETHODIMP get_HostRawElementProvider(IRawElementProviderSimple** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (!Valid()) return UIA_E_ELEMENTNOTAVAILABLE;
    return index_ < 0 ? UiaHostProviderFromHwnd(state_->hwnd, result) : S_OK;
  }
  STDMETHODIMP Navigate(NavigateDirection direction,
                        IRawElementProviderFragment** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (!Valid()) return UIA_E_ELEMENTNOTAVAILABLE;
    if (index_ < 0 && state_->visible && !state_->items.empty()) {
      if (direction == NavigateDirection_FirstChild) return Fragment(0, result);
      if (direction == NavigateDirection_LastChild)
        return Fragment(static_cast<int>(state_->items.size() - 1), result);
    } else if (index_ >= 0) {
      if (direction == NavigateDirection_Parent) return Fragment(-1, result);
      if (direction == NavigateDirection_NextSibling &&
          static_cast<size_t>(index_ + 1) < state_->items.size())
        return Fragment(index_ + 1, result);
      if (direction == NavigateDirection_PreviousSibling && index_ > 0)
        return Fragment(index_ - 1, result);
    }
    return S_OK;
  }
  STDMETHODIMP GetRuntimeId(SAFEARRAY** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (!Valid()) return UIA_E_ELEMENTNOTAVAILABLE;
    if (index_ < 0) return S_OK;  // HWND host supplies the fragment-root identity.
    int id[] = {UiaAppendRuntimeId, static_cast<int>(generation_), index_};
    *result = SafeArrayCreateVector(VT_I4, 0, ARRAYSIZE(id));
    if (!*result) return E_OUTOFMEMORY;
    for (LONG i = 0; i < ARRAYSIZE(id); ++i)
      SafeArrayPutElement(*result, &i, &id[i]);
    return S_OK;
  }
  STDMETHODIMP get_BoundingRectangle(UiaRect* result) override {
    if (!result) return E_POINTER;
    *result = {};
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (!Valid()) return UIA_E_ELEMENTNOTAVAILABLE;
    if (!state_->visible) return S_OK;
    RECT rect = index_ < 0 ? state_->bounds
                           : (static_cast<size_t>(index_) < state_->itemBounds.size()
                                  ? state_->itemBounds[index_]
                                  : RECT{});
    *result = {static_cast<double>(rect.left), static_cast<double>(rect.top),
               static_cast<double>(rect.right - rect.left),
               static_cast<double>(rect.bottom - rect.top)};
    return S_OK;
  }
  STDMETHODIMP GetEmbeddedFragmentRoots(SAFEARRAY** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    return S_OK;
  }
  STDMETHODIMP SetFocus() override { return UIA_E_NOTSUPPORTED; }  // Keep focus in the editor.
  STDMETHODIMP get_FragmentRoot(IRawElementProviderFragmentRoot** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (!Valid()) return UIA_E_ELEMENTNOTAVAILABLE;
    IRawElementProviderFragment* root = nullptr;
    HRESULT hr = Fragment(-1, &root);
    if (FAILED(hr)) return hr;
    hr = root->QueryInterface(__uuidof(IRawElementProviderFragmentRoot),
                              reinterpret_cast<void**>(result));
    root->Release();
    return hr;
  }
  STDMETHODIMP ElementProviderFromPoint(double x, double y,
                                        IRawElementProviderFragment** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (!Valid()) return UIA_E_ELEMENTNOTAVAILABLE;
    if (!state_->visible) return S_OK;
    for (size_t i = 0; i < state_->itemBounds.size(); ++i) {
      const RECT& rect = state_->itemBounds[i];
      if (x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom)
        return Fragment(static_cast<int>(i), result);
    }
    return Fragment(-1, result);
  }
  STDMETHODIMP GetFocus(IRawElementProviderFragment** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    return S_OK;
  }
  STDMETHODIMP GetSelection(SAFEARRAY** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (!Valid()) return UIA_E_ELEMENTNOTAVAILABLE;
    const bool hasSelection = state_->visible && state_->selection < state_->items.size();
    *result = SafeArrayCreateVector(VT_UNKNOWN, 0, hasSelection ? 1 : 0);
    if (!*result) return E_OUTOFMEMORY;
    if (hasSelection) {
      IRawElementProviderFragment* item = nullptr;
      HRESULT hr = Fragment(static_cast<int>(state_->selection), &item);
      if (FAILED(hr)) {
        SafeArrayDestroy(*result);
        *result = nullptr;
        return hr;
      }
      LONG zero = 0;
      hr = SafeArrayPutElement(*result, &zero, item);
      item->Release();
      return hr;
    }
    return S_OK;
  }
  STDMETHODIMP get_CanSelectMultiple(BOOL* result) override {
    if (!result) return E_POINTER;
    *result = FALSE;
    return S_OK;
  }
  STDMETHODIMP get_IsSelectionRequired(BOOL* result) override {
    if (!result) return E_POINTER;
    *result = TRUE;
    return S_OK;
  }
  STDMETHODIMP Select() override { return Action(SrfCandidateAccessibility::kSelectMessage); }
  STDMETHODIMP AddToSelection() override { return Select(); }
  STDMETHODIMP RemoveFromSelection() override { return UIA_E_INVALIDOPERATION; }
  STDMETHODIMP get_IsSelected(BOOL* result) override {
    if (!result) return E_POINTER;
    *result = FALSE;
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (!Valid()) return UIA_E_ELEMENTNOTAVAILABLE;
    *result = index_ >= 0 && static_cast<UINT>(index_) == state_->selection;
    return S_OK;
  }
  STDMETHODIMP get_SelectionContainer(IRawElementProviderSimple** result) override {
    if (!result) return E_POINTER;
    *result = nullptr;
    std::lock_guard<std::mutex> lock(state_->mutex);
    if (!Valid()) return UIA_E_ELEMENTNOTAVAILABLE;
    auto* root = new (std::nothrow) Provider(state_, -1, 0);
    if (!root) return E_OUTOFMEMORY;
    *result = root;
    return S_OK;
  }
  STDMETHODIMP Invoke() override { return Action(SrfCandidateAccessibility::kInvokeMessage); }
};

RECT PhysicalRect(HWND hwnd, RECT rect, bool client) {
  POINT points[2] = {{rect.left, rect.top}, {rect.right, rect.bottom}};
  if (client) {
    ClientToScreen(hwnd, &points[0]);
    ClientToScreen(hwnd, &points[1]);
  }
  LogicalToPhysicalPointForPerMonitorDPI(hwnd, &points[0]);
  LogicalToPhysicalPointForPerMonitorDPI(hwnd, &points[1]);
  return {points[0].x, points[0].y, points[1].x, points[1].y};
}
}  // namespace

SrfCandidateAccessibility::SrfCandidateAccessibility()
    : m_state(std::make_shared<SrfCandidateAccessibleState>()) {
}
SrfCandidateAccessibility::~SrfCandidateAccessibility() {
  Destroy();
}
LRESULT SrfCandidateAccessibility::GetObject(HWND hwnd, WPARAM wParam, LPARAM lParam) {
  if (static_cast<LONG>(lParam) != UiaRootObjectId) return 0;
  IRawElementProviderSimple* provider = nullptr;
  if (FAILED(ProviderForWindow(hwnd, &provider))) return 0;
  const LRESULT result = UiaReturnRawElementProvider(hwnd, wParam, lParam, provider);
  provider->Release();
  return result;
}
HRESULT SrfCandidateAccessibility::ProviderForWindow(HWND hwnd,
                                                     IRawElementProviderSimple** result) {
  if (!result) return E_POINTER;
  *result = nullptr;
  if (!m_state) m_state = std::make_shared<SrfCandidateAccessibleState>();
  if (!m_root) {
    {
      std::lock_guard<std::mutex> lock(m_state->mutex);
      m_state->hwnd = hwnd;
    }
    m_root = new (std::nothrow) Provider(m_state, -1, 0);
  }
  if (!m_root) return E_OUTOFMEMORY;
  m_root->AddRef();
  *result = m_root;
  return S_OK;
}
void SrfCandidateAccessibility::Update(HWND hwnd, const std::wstring& title,
                                       const std::vector<std::wstring>& items,
                                       const std::vector<std::wstring>& comments,
                                       const std::vector<RECT>& itemRects, UINT page,
                                       UINT selection, bool interactive) {
  if (!m_state) m_state = std::make_shared<SrfCandidateAccessibleState>();
  const bool visible = IsWindowVisible(hwnd) != FALSE;
  RECT bounds = {};
  GetWindowRect(hwnd, &bounds);
  bounds = PhysicalRect(hwnd, bounds, false);
  std::vector<RECT> rectangles;
  for (const RECT& rect : itemRects)
    rectangles.push_back(PhysicalRect(hwnd, rect, true));
  bool opened, changed, selected;
  ULONG generation;
  {
    std::lock_guard<std::mutex> lock(m_state->mutex);
    opened = visible && !m_state->visible;
    changed = m_state->items != items || m_state->page != page || m_state->title != title;
    selected = opened || changed || m_state->selection != selection;
    if (changed) ++m_state->generation;
    m_state->hwnd = hwnd;
    m_state->visible = visible;
    m_state->interactive = interactive;
    m_state->title = title;
    m_state->items = items;
    m_state->comments = comments;
    m_state->page = page;
    m_state->selection = selection;
    m_state->bounds = bounds;
    m_state->itemBounds = std::move(rectangles);
    generation = m_state->generation;
  }
  if (!visible) return;
  if (!m_root) m_root = new (std::nothrow) Provider(m_state, -1, 0);
  if (!m_root) return;
  if (opened) UiaRaiseAutomationEvent(m_root, UIA_MenuOpenedEventId);
  if (changed)
    UiaRaiseStructureChangedEvent(m_root, StructureChangeType_ChildrenInvalidated, nullptr, 0);
  if (selected && selection < items.size()) {
    auto* item = new (std::nothrow) Provider(m_state, static_cast<int>(selection), generation);
    if (item) {
      UiaRaiseAutomationEvent(item, UIA_SelectionItem_ElementSelectedEventId);
      item->Release();
    }
  }
}
void SrfCandidateAccessibility::Hide() {
  if (!m_state) return;
  bool closed;
  {
    std::lock_guard<std::mutex> lock(m_state->mutex);
    closed = m_state->visible;
    m_state->visible = false;
    m_state->interactive = false;
    if (closed) ++m_state->generation;
  }
  if (closed && m_root) UiaRaiseAutomationEvent(m_root, UIA_MenuClosedEventId);
}
void SrfCandidateAccessibility::Destroy() {
  if (!m_state) return;
  Hide();
  {
    std::lock_guard<std::mutex> lock(m_state->mutex);
    m_state->hwnd = nullptr;
    ++m_state->generation;
  }
  if (m_root) {
    UiaDisconnectProvider(m_root);
    m_root->Release();
    m_root = nullptr;
  }
  // A recreated HWND gets a fresh state. Retained old root providers must
  // never become valid again when a new composition/window appears.
  m_state.reset();
}
bool SrfCandidateAccessibility::CanAct(UINT index, ULONG generation) const {
  if (!m_state) return false;
  std::lock_guard<std::mutex> lock(m_state->mutex);
  return m_state->visible && m_state->interactive && generation == m_state->generation &&
         index < m_state->items.size();
}
