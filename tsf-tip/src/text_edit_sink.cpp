#include "text_edit_sink.h"
#include "srf_tip.h"
#include "tsf_edit_policy.h"
#include <new>
#include <string>
#include <utility>

extern void SrfTsfDiagnosticLog(const wchar_t* tag, const wchar_t* msg);

namespace {
bool RangeTextMatches(ITfRange* range, TfEditCookie cookie, const std::wstring& expected,
                      bool* readable = nullptr) {
  if (readable) *readable = false;
  if (!range || expected.size() > 32767) return false;
  std::wstring actual(expected.size() + 1, L'\0');
  ULONG count = 0;
  if (FAILED(range->GetText(cookie, 0, actual.data(), static_cast<ULONG>(actual.size()), &count)))
    return false;
  if (readable) *readable = true;
  actual.resize(count);
  return actual == expected;
}
}

// Holds the original TSF object, never resolves a new focused context and never
// submits text. It can therefore finish cleanup after a focus/session change.
class CEditSessionFinishComposition final : public ITfEditSession {
  LONG m_refs = 1;
  CSrfTip* m_tip;
  ITfComposition* m_composition;
  ITfContext* m_context;
  SrfFocusSnapshot m_focus;
  uint64_t m_generation;
  bool m_clear;
  SrfEditSessionOnce m_execution;
  std::wstring m_expected;
 public:
  CEditSessionFinishComposition(CSrfTip* tip, ITfComposition* composition,
                               ITfContext* context, bool clear, std::wstring expected)
      : m_tip(tip), m_composition(composition), m_context(context),
        m_focus(tip->CaptureFocusSnapshot(context)), m_generation(tip->m_compositionGeneration),
        m_clear(clear), m_expected(std::move(expected)) {
    m_tip->AddRef();
    m_composition->AddRef();
    m_context->AddRef();
  }
  ~CEditSessionFinishComposition() {
    m_composition->Release();
    m_context->Release();
    m_tip->Release();
  }
  STDMETHODIMP QueryInterface(REFIID riid, void** out) override {
    if (!out) return E_POINTER;
    *out = nullptr;
    if (riid != IID_IUnknown && riid != IID_ITfEditSession) return E_NOINTERFACE;
    *out = static_cast<ITfEditSession*>(this);
    AddRef();
    return S_OK;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_refs); }
  STDMETHODIMP_(ULONG) Release() override {
    const ULONG refs = InterlockedDecrement(&m_refs);
    if (!refs) delete this;
    return refs;
  }
  STDMETHODIMP DoEditSession(TfEditCookie cookie) override {
    if (!m_execution.Begin()) return S_OK;
    const bool sameSession = m_tip->m_compositionGeneration == m_generation &&
                             m_tip->FocusSnapshotMatches(m_focus);
    // If focus moved to another context, the captured object's range still
    // belongs to the old field. Never clear it when a newer composition is
    // active in that same context.
    const bool differentContext = m_tip->m_pFocusContext != m_context &&
                                  m_tip->m_pCompositionContext != m_context;
    if (m_clear && (sameSession || differentContext)) {
      ITfRange* range = nullptr;
      if (SUCCEEDED(m_composition->GetRange(&range)) && range) {
        // A host may have replaced the preedit since OnEndEdit. Preserve that
        // replacement and never delete a newer session's text.
        if (RangeTextMatches(range, cookie, m_expected)) (void)range->SetText(cookie, 0, L"", 0);
        range->Release();
      }
    }
    const SrfScopedCompositionEnd ending(m_tip->m_internalCompositionEndDepth);
    return m_composition->EndComposition(cookie);
  }
};

STDMETHODIMP CTextEditSink::QueryInterface(REFIID riid, void** out) {
  if (!out) return E_POINTER;
  *out = nullptr;
  if (riid == IID_IUnknown || riid == IID_ITfTextEditSink)
    *out = static_cast<ITfTextEditSink*>(this);
  else if (riid == IID_ITfTextLayoutSink)
    *out = static_cast<ITfTextLayoutSink*>(this);
  else return E_NOINTERFACE;
  AddRef();
  return S_OK;
}
STDMETHODIMP_(ULONG) CTextEditSink::AddRef() { return InterlockedIncrement(&m_refs); }
STDMETHODIMP_(ULONG) CTextEditSink::Release() {
  const ULONG refs = InterlockedDecrement(&m_refs);
  if (!refs) delete this;
  return refs;
}

STDMETHODIMP CTextEditSink::OnEndEdit(ITfContext* context, TfEditCookie cookie, ITfEditRecord* record) {
  if (!m_tip || !record || context != m_tip->m_pFocusContext ||
      context != m_tip->m_pCompositionContext || !m_tip->m_pComposition ||
      m_tip->m_internalCompositionEndDepth) return S_OK;
  ITfRange* compositionRange = nullptr;
  if (FAILED(m_tip->m_pComposition->GetRange(&compositionRange)) || !compositionRange) return S_OK;
  bool outside = false;
  BOOL selectionChanged = FALSE;
  if (SUCCEEDED(record->GetSelectionStatus(&selectionChanged)) && selectionChanged) {
    TF_SELECTION selection = {};
    ULONG fetched = 0;
    if (SUCCEEDED(context->GetSelection(cookie, TF_DEFAULT_SELECTION, 1, &selection, &fetched)) &&
        fetched == 1 && selection.range) {
      LONG start = 0, end = 0;
      const HRESULT startHr = selection.range->CompareStart(cookie, compositionRange, TF_ANCHOR_START, &start);
      const HRESULT endHr = selection.range->CompareEnd(cookie, compositionRange, TF_ANCHOR_END, &end);
      outside = SrfSelectionOutsideComposition(startHr, start, endHr, end);
    }
    if (selection.range) selection.range->Release();
  }
  bool replaced = false;
  IEnumTfRanges* changes = nullptr;
  if (SUCCEEDED(record->GetTextAndPropertyUpdates(TF_GTP_INCL_TEXT, nullptr, 0, &changes)) && changes) {
    ITfRange* changed = nullptr;
    ULONG fetched = 0;
    if (changes->Next(1, &changed, &fetched) == S_OK && fetched == 1) {
      bool readable = false;
      const bool matches = RangeTextMatches(compositionRange, cookie,
                                           m_tip->BuildCompositionDisplay(), &readable);
      replaced = readable && !matches;
    }
    if (changed) changed->Release();
  }
  if (changes) changes->Release();
  compositionRange->Release();
  if (outside || replaced) {
    // OnEndEdit only grants a read lock. Queue the write cleanup; never reuse
    // this cookie for SetText/EndComposition.
    m_tip->ScheduleHostCompositionEnd(!replaced);
  }
  return S_OK;
}

STDMETHODIMP CTextEditSink::OnLayoutChange(ITfContext* context, TfLayoutCode code, ITfContextView*) {
  if (!m_tip || context != m_tip->m_pFocusContext || context != m_tip->m_pCompositionContext ||
      !m_tip->m_pComposition) return S_OK;
  if (code == TF_LC_DESTROY) {
    m_tip->ScheduleHostCompositionEnd(false);
  } else if (code == TF_LC_CHANGE) {
    m_tip->m_hasStickyCandidateRect = false;
    m_tip->m_hasLastCandidateRect = false;
    if (!m_tip->RequestCandidateWindowAnchorRefresh()) m_tip->ScheduleCandidateWindowAnchorRefreshRetry();
  }
  return S_OK;
}

void CSrfTip::UnbindTextContextSinks() {
  if (m_pTextEditSink) m_pTextEditSink->Detach();
  if (m_pTextEditSource) {
    if (m_textEditCookie != TF_INVALID_COOKIE) (void)m_pTextEditSource->UnadviseSink(m_textEditCookie);
    if (m_textLayoutCookie != TF_INVALID_COOKIE) (void)m_pTextEditSource->UnadviseSink(m_textLayoutCookie);
    m_pTextEditSource->Release();
    m_pTextEditSource = nullptr;
  }
  m_textEditCookie = m_textLayoutCookie = TF_INVALID_COOKIE;
  if (m_pTextEditContext) { m_pTextEditContext->Release(); m_pTextEditContext = nullptr; }
  if (m_pTextEditSink) { m_pTextEditSink->Release(); m_pTextEditSink = nullptr; }
}

void CSrfTip::BindTextContextSinks(ITfContext* context) {
  if (context == m_pTextEditContext) return;
  UnbindTextContextSinks();
  if (!context) return;
  if (FAILED(context->QueryInterface(IID_ITfSource, reinterpret_cast<void**>(&m_pTextEditSource))) ||
      !m_pTextEditSource) return;
  m_pTextEditSink = new (std::nothrow) CTextEditSink(this);
  if (!m_pTextEditSink) { UnbindTextContextSinks(); return; }
  m_pTextEditContext = context;
  context->AddRef();
  if (FAILED(m_pTextEditSource->AdviseSink(IID_ITfTextEditSink,
      static_cast<ITfTextEditSink*>(m_pTextEditSink), &m_textEditCookie))) m_textEditCookie = TF_INVALID_COOKIE;
  if (FAILED(m_pTextEditSource->AdviseSink(IID_ITfTextLayoutSink,
      static_cast<ITfTextLayoutSink*>(m_pTextEditSink), &m_textLayoutCookie))) m_textLayoutCookie = TF_INVALID_COOKIE;
  if (m_textEditCookie == TF_INVALID_COOKIE && m_textLayoutCookie == TF_INVALID_COOKIE) UnbindTextContextSinks();
}

void CSrfTip::ScheduleHostCompositionEnd(bool clearPreedit) {
  if (!m_pComposition || !m_pCompositionContext) return;
  const std::wstring expected = BuildCompositionDisplay();
  ITfComposition* composition = m_pComposition;
  ITfContext* context = m_pCompositionContext;
  composition->AddRef();
  context->AddRef();
  EndInputSession(L"host-text-or-layout-change", TF_INVALID_COOKIE, false);
  ReleaseCompositionObjects();
  auto* edit = new (std::nothrow) CEditSessionFinishComposition(this, composition, context, clearPreedit, expected);
  if (edit) {
    HRESULT session = E_FAIL;
    const HRESULT request = context->RequestEditSession(m_tid, edit, TF_ES_ASYNC | TF_ES_READWRITE, &session);
    if (FAILED(request) || FAILED(session))
      SrfTsfDiagnosticLog(L"composition-end.rejected", L"host refused asynchronous cleanup; input session already invalidated");
    edit->Release();
  }
  context->Release();
  composition->Release();
}

HRESULT CSrfTip::FinishCompositionPreservingText(TfEditCookie cookie) {
  if (!m_pComposition || !m_pCompositionContext) return S_OK;
  ITfComposition* composition = m_pComposition;
  ITfContext* context = m_pCompositionContext;
  composition->AddRef();
  context->AddRef();
  // Detach the old identity before a host can synchronously call the sink.
  ReleaseCompositionObjects();
  ++m_compositionGeneration;
  HRESULT finish;
  {
    const SrfScopedCompositionEnd ending(m_internalCompositionEndDepth);
    finish = composition->EndComposition(cookie);
  }
  if (FAILED(finish)) {
    auto* edit = new (std::nothrow) CEditSessionFinishComposition(this, composition, context, false, L"");
    if (edit) {
      HRESULT session = E_FAIL;
      const HRESULT request = context->RequestEditSession(m_tid, edit, TF_ES_ASYNC | TF_ES_READWRITE, &session);
      if (FAILED(request) || FAILED(session))
        SrfTsfDiagnosticLog(L"composition-end.rejected", L"host refused cleanup retry; text will not be resubmitted");
      edit->Release();
    }
    SrfTsfDiagnosticLog(L"composition-end.retry", L"attempted cleanup without resubmitting text");
  }
  context->Release();
  composition->Release();
  return finish;
}
