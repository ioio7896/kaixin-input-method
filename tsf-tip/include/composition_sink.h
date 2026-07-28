#pragma once
#include <msctf.h>

class CSrfTip;

// TSF ITfCompositionSink：组合结束时回收 ITfComposition 引用与缓冲区
class CCompositionSink final : public ITfCompositionSink {
  LONG m_cRef = 1;
  CSrfTip* m_pTip = nullptr;

 public:
  explicit CCompositionSink(CSrfTip* tip);

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;
  STDMETHODIMP OnCompositionTerminated(TfEditCookie ecWrite, ITfComposition* pComposition) override;
};
