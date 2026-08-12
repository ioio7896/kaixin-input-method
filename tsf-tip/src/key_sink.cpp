#include "key_sink.h"

#include <cstdio>
#include <filesystem>
#include <new>
#include <shellapi.h>
#include <string>
#include <system_error>

#include "guids.h"
#include "srf_tip.h"

// 由 srf_tip.cpp 中的匿名命名空间提供。
extern bool SrfTsfDebugTraceEnabled();
extern void SrfTsfDebugLog(const wchar_t* msg);
extern "C" IMAGE_DOS_HEADER __ImageBase;

namespace {

bool IsVkShift(UINT vk) { return vk == VK_SHIFT || vk == VK_LSHIFT || vk == VK_RSHIFT; }

bool IsKeyDownNow(int vk) {
  return (GetKeyState(vk) & 0x8000) != 0 || (GetAsyncKeyState(vk) & 0x8000) != 0;
}

bool CaptureShiftDown(UINT vk, LPARAM lParam, bool trackedShiftDown) {
  if (IsVkShift(vk)) {
    const bool keyUp = (lParam & 0x80000000) != 0;
    return !keyUp;
  }
  return trackedShiftDown || IsKeyDownNow(VK_SHIFT) || IsKeyDownNow(VK_LSHIFT) ||
         IsKeyDownNow(VK_RSHIFT);
}

UINT ResolveShiftVk(UINT vk, LPARAM lParam) {
  if (vk != VK_SHIFT) return vk;
  const UINT scanCode = HIWORD(static_cast<DWORD>(lParam)) & 0xff;
  if (scanCode == MapVirtualKeyW(VK_LSHIFT, MAPVK_VK_TO_VSC)) return VK_LSHIFT;
  if (scanCode == MapVirtualKeyW(VK_RSHIFT, MAPVK_VK_TO_VSC)) return VK_RSHIFT;
  return VK_SHIFT;
}

void UpdateTrackedShiftState(UINT vk, LPARAM lParam, bool down, bool* leftShiftDown,
                             bool* rightShiftDown) {
  if (!leftShiftDown || !rightShiftDown) return;
  switch (ResolveShiftVk(vk, lParam)) {
    case VK_LSHIFT:
      *leftShiftDown = down;
      break;
    case VK_RSHIFT:
      *rightShiftDown = down;
      break;
    case VK_SHIFT:
      *leftShiftDown = down;
      *rightShiftDown = down;
      break;
    default:
      break;
  }
}

void LaunchSystemScreenshot() {
  wchar_t modulePath[MAX_PATH] = {};
  const DWORD len = GetModuleFileNameW(reinterpret_cast<HMODULE>(&__ImageBase), modulePath,
                                       static_cast<DWORD>(_countof(modulePath)));
  if (len > 0 && len < _countof(modulePath)) {
    std::filesystem::path probe = std::filesystem::path(modulePath).parent_path();
    for (int i = 0; i < 6 && !probe.empty(); ++i) {
      const std::filesystem::path trayPath = probe / L"srf_ime_tray.exe";
      std::error_code trayEc;
      if (std::filesystem::is_regular_file(trayPath, trayEc)) {
        const HWND targetHwnd = GetForegroundWindow();
        wchar_t params[128] = {};
        swprintf_s(params, L"--screenshot-capture --target-hwnd %Id",
                   reinterpret_cast<INT_PTR>(targetHwnd));
        HINSTANCE helperResult =
            ShellExecuteW(nullptr, L"open", trayPath.c_str(), params, probe.c_str(), SW_SHOWNORMAL);
        if (reinterpret_cast<INT_PTR>(helperResult) > 32) return;
      }

      const std::filesystem::path shareXPath = probe / L"ShareX" / L"KaixinShareX.exe";
      std::error_code shareXEc;
      if (std::filesystem::is_regular_file(shareXPath, shareXEc)) {
        HINSTANCE shareXResult = ShellExecuteW(
            nullptr, L"open", shareXPath.c_str(),
            L"-portable -silent -KaixinRectangleRegion", probe.c_str(), SW_SHOWNORMAL);
        if (reinterpret_cast<INT_PTR>(shareXResult) > 32) return;
      }
      probe = probe.parent_path();
    }
  }

  if (SrfTsfDebugTraceEnabled()) {
    SrfTsfDebugLog(L"Failed to launch packaged ShareX screenshot workflow");
  }
}

class CEditSessionProcessKey final : public ITfEditSession {
  LONG m_cRef = 1;
  CSrfTip* m_pTip = nullptr;
  ITfContext* m_pic = nullptr;
  UINT m_vk = 0;
  LPARAM m_lParam = 0;
  bool m_shiftDown = false;
  bool m_handled = false;

 public:
  CEditSessionProcessKey(CSrfTip* tip, ITfContext* pic, UINT vk, LPARAM lParam, bool shiftDown)
      : m_pTip(tip), m_vk(vk), m_lParam(lParam), m_shiftDown(shiftDown) {
    m_pic = pic;
    if (m_pic) m_pic->AddRef();
  }
  ~CEditSessionProcessKey() {
    if (m_pic) m_pic->Release();
  }

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (riid == IID_IUnknown || riid == IID_ITfEditSession) {
      *ppv = static_cast<ITfEditSession*>(this);
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }
  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_cRef); }
  STDMETHODIMP_(ULONG) Release() override {
    ULONG c = InterlockedDecrement(&m_cRef);
    if (c == 0) delete this;
    return c;
  }

  STDMETHODIMP DoEditSession(TfEditCookie ec) override {
    if (!m_pTip || !m_pic) return E_FAIL;
    m_handled = false;
    return m_pTip->ProcessKey(ec, m_pic, m_vk, m_lParam, m_shiftDown, &m_handled);
  }

  bool Consumed() const { return m_handled; }
};

/// 请求编辑会话，先 SYNC 后 ASYNC 回退。
/// 返回 true 表示按键已被（或将被）消费。
bool RequestEditSessionWithFallback(ITfContext* pic, TfClientId tid,
                                    CEditSessionProcessKey* pEdit) {
  HRESULT hrSession = E_FAIL;
  HRESULT hr = pic->RequestEditSession(tid, pEdit, TF_ES_SYNC | TF_ES_READWRITE, &hrSession);
  if (SUCCEEDED(hr) && SUCCEEDED(hrSession)) {
    return pEdit->Consumed();
  }

  // 同步请求失败 — 回退到异步。
  // 异步模式下 DoEditSession 尚未执行，无法通过 Consumed() 判断结果，
  // 但 OnTestKeyDown 已承诺吃掉该键，因此预设 consumed = true。
  if (SrfTsfDebugTraceEnabled()) {
    wchar_t buf[120] = {};
    swprintf_s(buf, L"EditSession SYNC failed hr=0x%08lX session=0x%08lX, falling back to ASYNC",
               static_cast<unsigned long>(hr), static_cast<unsigned long>(hrSession));
    SrfTsfDebugLog(buf);
  }
  hrSession = E_FAIL;
  hr = pic->RequestEditSession(tid, pEdit, TF_ES_ASYNC | TF_ES_READWRITE, &hrSession);
  if (SUCCEEDED(hr) && SUCCEEDED(hrSession)) {
    return true;  // 异步已入队，预设消费
  }

  // SYNC + ASYNC 均失败
  if (SrfTsfDebugTraceEnabled()) {
    wchar_t buf[120] = {};
    swprintf_s(buf, L"EditSession ASYNC also failed hr=0x%08lX session=0x%08lX",
               static_cast<unsigned long>(hr), static_cast<unsigned long>(hrSession));
    SrfTsfDebugLog(buf);
  }
  return false;
}

}  // namespace

CKeyEventSink::CKeyEventSink(CSrfTip* tip) : m_pTip(tip) {}

STDMETHODIMP CKeyEventSink::QueryInterface(REFIID riid, void** ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;
  if (riid == IID_IUnknown || riid == IID_ITfKeyEventSink) {
    *ppv = static_cast<ITfKeyEventSink*>(this);
    AddRef();
    return S_OK;
  }
  return E_NOINTERFACE;
}

STDMETHODIMP_(ULONG) CKeyEventSink::AddRef() { return InterlockedIncrement(&m_cRef); }

STDMETHODIMP_(ULONG) CKeyEventSink::Release() {
  ULONG c = InterlockedDecrement(&m_cRef);
  if (c == 0) delete this;
  return c;
}

STDMETHODIMP CKeyEventSink::OnSetFocus(BOOL fForeground) {
  if (!fForeground) {
    m_leftShiftDown = false;
    m_rightShiftDown = false;
  }
  return S_OK;
}

STDMETHODIMP CKeyEventSink::OnTestKeyDown(ITfContext* /*pic*/, WPARAM wParam, LPARAM lParam,
                                          BOOL* pfEaten) {
  if (!pfEaten) return E_POINTER;
  *pfEaten = FALSE;
  if (!m_pTip) return S_OK;
  const UINT vk = static_cast<UINT>(wParam);
  if (IsVkShift(vk)) {
    UpdateTrackedShiftState(vk, lParam, true, &m_leftShiftDown, &m_rightShiftDown);
  }
  if (m_pTip->WouldEatKey(vk)) *pfEaten = TRUE;
  return S_OK;
}

STDMETHODIMP CKeyEventSink::OnTestKeyUp(ITfContext* /*pic*/, WPARAM wParam, LPARAM lParam,
                                        BOOL* pfEaten) {
  if (!pfEaten) return E_POINTER;
  *pfEaten = FALSE;
  if (!m_pTip) return S_OK;
  const UINT vk = static_cast<UINT>(wParam);
  // Only Shift KeyUp is handled by the IME. If it will be handled, keep the
  // tracked state until OnKeyUp consumes the real release event.
  if (IsVkShift(vk)) {
    const bool wouldEat = m_pTip->WouldEatKey(vk);
    if (wouldEat) {
      *pfEaten = TRUE;
    } else {
      UpdateTrackedShiftState(vk, lParam, false, &m_leftShiftDown, &m_rightShiftDown);
    }
  }
  return S_OK;
}

STDMETHODIMP CKeyEventSink::OnKeyDown(ITfContext* pic, WPARAM wParam, LPARAM lParam,
                                      BOOL* pfEaten) {
  if (!pfEaten) return E_POINTER;
  *pfEaten = FALSE;
  if (!pic || !m_pTip) return S_OK;

  const UINT vk = static_cast<UINT>(wParam);
  if (IsVkShift(vk)) {
    UpdateTrackedShiftState(vk, lParam, true, &m_leftShiftDown, &m_rightShiftDown);
  }
  if (!m_pTip->WouldEatKey(vk)) return S_OK;

  const bool shiftDown = CaptureShiftDown(vk, lParam, m_leftShiftDown || m_rightShiftDown);
  CEditSessionProcessKey* pEdit =
      new (std::nothrow) CEditSessionProcessKey(m_pTip, pic, vk, lParam, shiftDown);
  if (!pEdit) return S_OK;

  const bool consumed = RequestEditSessionWithFallback(pic, m_pTip->m_tid, pEdit);
  pEdit->Release();

  if (consumed) *pfEaten = TRUE;
  return S_OK;
}

STDMETHODIMP CKeyEventSink::OnKeyUp(ITfContext* pic, WPARAM wParam, LPARAM lParam, BOOL* pfEaten) {
  if (!pfEaten) return E_POINTER;
  *pfEaten = FALSE;
  if (!pic || !m_pTip) return S_OK;

  const UINT vk = static_cast<UINT>(wParam);
  if (!IsVkShift(vk)) return S_OK;
  const bool shiftDown = CaptureShiftDown(vk, lParam, m_leftShiftDown || m_rightShiftDown);
  UpdateTrackedShiftState(vk, lParam, false, &m_leftShiftDown, &m_rightShiftDown);
  if (!m_pTip->WouldEatKey(vk)) return S_OK;

  CEditSessionProcessKey* pEdit =
      new (std::nothrow) CEditSessionProcessKey(m_pTip, pic, vk, lParam, shiftDown);
  if (!pEdit) return S_OK;

  const bool consumed = RequestEditSessionWithFallback(pic, m_pTip->m_tid, pEdit);
  pEdit->Release();

  if (consumed) *pfEaten = TRUE;
  return S_OK;
}

STDMETHODIMP CKeyEventSink::OnPreservedKey(ITfContext* /*pic*/, REFGUID rguid, BOOL* pfEaten) {
  if (!pfEaten) return E_POINTER;
  *pfEaten = FALSE;
  if (!m_pTip) return S_OK;

  const bool hasReading = m_pTip->m_imeOpen && !m_pTip->m_reading.empty();
  if (IsEqualGUID(rguid, GUID_PRESERVEDKEY_SRF_TOGGLE_IME) ||
      IsEqualGUID(rguid, GUID_PRESERVEDKEY_SRF_TOGGLE_IME_CTRL_SPACE)) {
    if (IsEqualGUID(rguid, GUID_PRESERVEDKEY_SRF_TOGGLE_IME) && !hasReading) {
      return S_OK;
    }
    if (m_pTip->ShouldSuppressImeTogglePreservedKey()) {
      *pfEaten = TRUE;
      return S_OK;
    }
    if (hasReading) {
      const HRESULT hrCommit = m_pTip->RequestCommitReadingText();
      if (FAILED(hrCommit)) {
        *pfEaten = TRUE;
        return hrCommit;
      }
    }
    m_pTip->ToggleImeOpen();
  } else if (IsEqualGUID(rguid, GUID_PRESERVEDKEY_SRF_TOGGLE_FULLSHAPE)) {
    m_pTip->ToggleFullShape();
  } else if (IsEqualGUID(rguid, GUID_PRESERVEDKEY_SRF_TOGGLE_PUNCT)) {
    if (!hasReading) return S_OK;
    m_pTip->ToggleChinesePunctuation();
  } else if (IsEqualGUID(rguid, GUID_PRESERVEDKEY_SRF_TOGGLE_FUZZY)) {
    if (!hasReading) return S_OK;
    m_pTip->ToggleFuzzyPinyin();
  } else if (IsEqualGUID(rguid, GUID_PRESERVEDKEY_SRF_TOGGLE_DOUBLE)) {
    if (!hasReading) return S_OK;
    m_pTip->ToggleDoublePinyin();
  } else if (IsEqualGUID(rguid, GUID_PRESERVEDKEY_SRF_SCREENSHOT)) {
    LaunchSystemScreenshot();
  } else {
    return S_OK;
  }

  *pfEaten = TRUE;
  return S_OK;
}
