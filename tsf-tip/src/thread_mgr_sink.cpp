#include "thread_mgr_sink.h"
#include "srf_tip.h"

#include <cstdio>

// 由 srf_tip.cpp 提供。
extern bool SrfTsfDebugTraceEnabled();
extern void SrfTsfDebugLog(const wchar_t* msg);

namespace {
bool ContextBelongsToDocument(ITfContext* context, ITfDocumentMgr* document) {
  if (!context || !document) return false;
  ITfDocumentMgr* owner = nullptr;
  const bool matches = SUCCEEDED(context->GetDocumentMgr(&owner)) && owner == document;
  if (owner) owner->Release();
  return matches;
}
}

CThreadMgrEventSink::CThreadMgrEventSink(CSrfTip* tip) : m_pTip(tip) {}

STDMETHODIMP CThreadMgrEventSink::QueryInterface(REFIID riid, void** ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;
  if (riid == IID_IUnknown || riid == IID_ITfThreadMgrEventSink) {
    *ppv = static_cast<ITfThreadMgrEventSink*>(this);
    AddRef();
    return S_OK;
  }
  return E_NOINTERFACE;
}

STDMETHODIMP_(ULONG) CThreadMgrEventSink::AddRef() { return InterlockedIncrement(&m_cRef); }

STDMETHODIMP_(ULONG) CThreadMgrEventSink::Release() {
  ULONG c = InterlockedDecrement(&m_cRef);
  if (c == 0) delete this;
  return c;
}

STDMETHODIMP CThreadMgrEventSink::OnInitDocumentMgr(ITfDocumentMgr* /*pdim*/) { return S_OK; }

STDMETHODIMP CThreadMgrEventSink::OnUninitDocumentMgr(ITfDocumentMgr* pdim) {
  if (!m_pTip || !pdim) return S_OK;
  const bool focused = ContextBelongsToDocument(m_pTip->m_pFocusContext, pdim);
  if (ContextBelongsToDocument(m_pTip->m_pCompositionContext, pdim)) {
    m_pTip->ScheduleHostCompositionEnd(false);
  }
  if (focused) m_pTip->SetFocusContext(nullptr);
  return S_OK;
}

STDMETHODIMP CThreadMgrEventSink::OnSetFocus(ITfDocumentMgr* pdimFocus, ITfDocumentMgr* pdimPrevFocus) {
  const ULONGLONG start = GetTickCount64();
  if (m_pTip) {
    const bool focusChanged =
        pdimFocus != pdimPrevFocus && (pdimFocus != nullptr || pdimPrevFocus != nullptr);
    const bool deferredNullFocusClear =
        !pdimFocus && focusChanged && m_pTip->ScheduleDeferredFocusContextClear();
    // SetFocusContext owns composition cleanup after the transient-null grace.
    if (deferredNullFocusClear) {
      // Some hosts briefly report a null TSF focus while the same text field is still active.
      // Keep the current context alive for one short grace window; a real loss is handled by
      // the deferred timer.
    } else if (pdimFocus) {
      m_pTip->CancelDeferredFocusContextClear();
      ITfContext* pTop = nullptr;
      if (SUCCEEDED(pdimFocus->GetTop(&pTop)) && pTop) {
        m_pTip->SetFocusContext(pTop);
        pTop->Release();
        m_pTip->ApplyAppOptionsForFocusedContext(pdimPrevFocus && pdimFocus != pdimPrevFocus);
      } else {
        m_pTip->SetFocusContext(nullptr);
      }
    } else {
      m_pTip->CancelDeferredFocusContextClear();
      m_pTip->SetFocusContext(nullptr);
    }
  }
  if (SrfTsfDebugTraceEnabled()) {
    wchar_t buf[180] = {};
    swprintf_s(buf, L"[perf] ThreadMgrEvent/OnSetFocus total=%llums focus=%p prev=%p changed=%d",
               static_cast<unsigned long long>(GetTickCount64() - start),
               static_cast<const void*>(pdimFocus), static_cast<const void*>(pdimPrevFocus),
               pdimFocus != pdimPrevFocus ? 1 : 0);
    SrfTsfDebugLog(buf);
  }
  return S_OK;
}

STDMETHODIMP CThreadMgrEventSink::OnPushContext(ITfContext* pic) {
  if (m_pTip && pic) {
    ITfDocumentMgr* focused = nullptr;
    if (m_pTip->m_pThreadMgr && SUCCEEDED(m_pTip->m_pThreadMgr->GetFocus(&focused)) && focused) {
      if (ContextBelongsToDocument(pic, focused)) {
        m_pTip->SetFocusContext(pic);
        m_pTip->ApplyAppOptionsForFocusedContext(false);
      }
    }
    if (focused) focused->Release();
  }
  return S_OK;
}

STDMETHODIMP CThreadMgrEventSink::OnPopContext(ITfContext* pic) {
  if (!m_pTip || !pic) return S_OK;
  const bool focused = m_pTip->m_pFocusContext == pic;
  if (m_pTip->m_pCompositionContext == pic) m_pTip->ScheduleHostCompositionEnd(false);
  if (!focused) return S_OK;
  m_pTip->SetFocusContext(nullptr);
  ITfDocumentMgr* document = nullptr;
  if (m_pTip->m_pThreadMgr && SUCCEEDED(m_pTip->m_pThreadMgr->GetFocus(&document)) && document) {
    ITfContext* top = nullptr;
    if (SUCCEEDED(document->GetTop(&top)) && top && top != pic) {
      m_pTip->SetFocusContext(top);
      m_pTip->ApplyAppOptionsForFocusedContext(false);
    }
    if (top) top->Release();
  }
  if (document) document->Release();
  return S_OK;
}
