#pragma once
#include <msctf.h>
#include "game_key_ownership.h"

class CSrfTip;

struct CKeyEventSink : public ITfKeyEventSink, public ITfKeyTraceEventSink {
  LONG m_cRef = 1;
  CSrfTip* m_pTip = nullptr;
  bool m_leftShiftDown = false;
  bool m_rightShiftDown = false;
  SrfGameKeyOwnership m_keyOwnership;
  bool m_traceAvailable = false;
  bool m_gameToggleLatched = false;
  bool m_asciiToggleLatched = false;
  bool m_gameToggleRepeated = false;
  bool m_asciiToggleRepeated = false;
  SrfPendingKeyEdits m_pendingKeyEdits;
  void RefreshToggleLatches();

  explicit CKeyEventSink(CSrfTip* tip);
  void ResetKeyState();

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
  STDMETHODIMP_(ULONG) AddRef() override;
  STDMETHODIMP_(ULONG) Release() override;

  STDMETHODIMP OnSetFocus(BOOL fForeground) override;
  STDMETHODIMP OnTestKeyDown(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
  STDMETHODIMP OnTestKeyUp(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
  STDMETHODIMP OnKeyDown(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
  STDMETHODIMP OnKeyUp(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) override;
  STDMETHODIMP OnPreservedKey(ITfContext* pic, REFGUID rguid, BOOL* pfEaten) override;
  STDMETHODIMP OnKeyTraceDown(WPARAM wParam, LPARAM lParam) override;
  STDMETHODIMP OnKeyTraceUp(WPARAM wParam, LPARAM lParam) override;
};
