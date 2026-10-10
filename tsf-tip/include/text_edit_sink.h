#pragma once
#include <msctf.h>

class CSrfTip;

class CTextEditSink final : public ITfTextEditSink, public ITfTextLayoutSink {
  LONG m_refs = 1;
  CSrfTip* m_tip = nullptr;
 public:
  explicit CTextEditSink(CSrfTip* tip) : m_tip(tip) {}
  void Detach() { m_tip = nullptr; }
  STDMETHODIMP QueryInterface(REFIID riid, void** out) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;
  STDMETHODIMP OnEndEdit(ITfContext* context, TfEditCookie cookie, ITfEditRecord* record) override;
  STDMETHODIMP OnLayoutChange(ITfContext* context, TfLayoutCode code, ITfContextView* view) override;
};
