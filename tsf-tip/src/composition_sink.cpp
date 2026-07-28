#include "composition_sink.h"
#include "srf_tip.h"

CCompositionSink::CCompositionSink(CSrfTip* tip) : m_pTip(tip) {}

STDMETHODIMP CCompositionSink::QueryInterface(REFIID riid, void** ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;
  if (riid == IID_IUnknown || riid == IID_ITfCompositionSink) {
    *ppv = static_cast<ITfCompositionSink*>(this);
    AddRef();
    return S_OK;
  }
  return E_NOINTERFACE;
}

STDMETHODIMP_(ULONG) CCompositionSink::AddRef() { return InterlockedIncrement(&m_cRef); }

STDMETHODIMP_(ULONG) CCompositionSink::Release() {
  ULONG c = InterlockedDecrement(&m_cRef);
  if (c == 0) delete this;
  return c;
}

STDMETHODIMP CCompositionSink::OnCompositionTerminated(TfEditCookie /*ecWrite*/,
                                                       ITfComposition* pComposition) {
  if (!m_pTip) return S_OK;
  if (m_pTip->m_pComposition == pComposition) {
    m_pTip->m_pComposition->Release();
    m_pTip->m_pComposition = nullptr;
  }
  m_pTip->ReleaseCompositionState();
  return S_OK;
}
