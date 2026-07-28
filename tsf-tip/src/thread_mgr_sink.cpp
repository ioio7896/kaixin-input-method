#include "thread_mgr_sink.h"
#include "srf_tip.h"

#include <cstdio>

// 由 srf_tip.cpp 提供。
extern bool SrfTsfDebugTraceEnabled();
extern void SrfTsfDebugLog(const wchar_t* msg);

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

STDMETHODIMP CThreadMgrEventSink::OnUninitDocumentMgr(ITfDocumentMgr* /*pdim*/) { return S_OK; }

STDMETHODIMP CThreadMgrEventSink::OnSetFocus(ITfDocumentMgr* pdimFocus, ITfDocumentMgr* pdimPrevFocus) {
  const ULONGLONG start = GetTickCount64();
  if (m_pTip) {
    const bool focusChanged =
        pdimFocus != pdimPrevFocus && (pdimFocus != nullptr || pdimPrevFocus != nullptr);
    const bool deferredNullFocusClear =
        !pdimFocus && focusChanged && m_pTip->ScheduleDeferredFocusContextClear();
    // 当文档管理器切换时取消未完成的组合。
    // 放宽条件：pdimPrevFocus 可能为 nullptr（部分宿主如 WebView2、Java 应用），
    // 此时只要焦点确实变了（pdimFocus != pdimPrevFocus）就应取消组合，
    // 但 pdimFocus == nullptr 且 pdimPrevFocus == nullptr 的情况排除（无意义的空切换）。
    if (!deferredNullFocusClear && m_pTip->m_pComposition && focusChanged) {
      m_pTip->RequestCancelCompositionOnFocusLoss();
    }
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
    m_pTip->SetFocusContext(pic);
    m_pTip->ApplyAppOptionsForFocusedContext(false);
  }
  return S_OK;
}

STDMETHODIMP CThreadMgrEventSink::OnPopContext(ITfContext* /*pic*/) { return S_OK; }
