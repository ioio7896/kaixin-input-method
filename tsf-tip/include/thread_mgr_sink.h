#pragma once
#include <msctf.h>

class CSrfTip;

struct CThreadMgrEventSink : public ITfThreadMgrEventSink {
  LONG m_cRef = 1;
  CSrfTip* m_pTip = nullptr;

  explicit CThreadMgrEventSink(CSrfTip* tip);

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;

  STDMETHODIMP OnInitDocumentMgr(ITfDocumentMgr* pdim) override;
  STDMETHODIMP OnUninitDocumentMgr(ITfDocumentMgr* pdim) override;
  STDMETHODIMP OnSetFocus(ITfDocumentMgr* pdimFocus, ITfDocumentMgr* pdimPrevFocus) override;
  STDMETHODIMP OnPushContext(ITfContext* pic) override;
  STDMETHODIMP OnPopContext(ITfContext* pic) override;
};
