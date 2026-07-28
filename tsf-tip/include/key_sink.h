#pragma once
#include <msctf.h>

class CSrfTip;

struct CKeyEventSink : public ITfKeyEventSink {
  LONG m_cRef = 1;
  CSrfTip* m_pTip = nullptr;
  bool m_leftShiftDown = false;
  bool m_rightShiftDown = false;

  explicit CKeyEventSink(CSrfTip* tip);

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;

  STDMETHODIMP OnSetFocus(BOOL fForeground) override;
  STDMETHODIMP OnTestKeyDown(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
  STDMETHODIMP OnTestKeyUp(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
  STDMETHODIMP OnKeyDown(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
  STDMETHODIMP OnKeyUp(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
  STDMETHODIMP OnPreservedKey(ITfContext* pic, REFGUID rguid, BOOL* pfEaten) override;
};
