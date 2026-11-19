#include <windows.h>
#include <unknwn.h>
#include <msctf.h>
#include <new>

#include "guids.h"
#include "srf_tip.h"

extern "C" void SrfTip_LockServer(BOOL lock);

namespace {

class CClassFactory : public IClassFactory {
  LONG m_cRef = 1;

 public:
  CClassFactory() = default;

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (riid == IID_IUnknown || riid == IID_IClassFactory) {
      *ppv = static_cast<IClassFactory*>(this);
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }

  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_cRef); }

  STDMETHODIMP_(ULONG) Release() override {
    const ULONG count = InterlockedDecrement(&m_cRef);
    if (count == 0) delete this;
    return count;
  }

  STDMETHODIMP CreateInstance(IUnknown* pUnkOuter, REFIID riid, void** ppv) override try {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (pUnkOuter) return CLASS_E_NOAGGREGATION;

    CSrfTip* p = new (std::nothrow) CSrfTip();
    if (!p) return E_OUTOFMEMORY;

    HRESULT hr = p->QueryInterface(riid, ppv);
    p->Release();
    return hr;
  } catch (const std::bad_alloc&) {
    return E_OUTOFMEMORY;
  } catch (...) {
    return E_FAIL;
  }

  STDMETHODIMP LockServer(BOOL fLock) override {
    SrfTip_LockServer(fLock);
    return S_OK;
  }
};

}  // namespace

HRESULT SrfTip_DllGetClassObject(REFCLSID rclsid, REFIID riid, void** ppv) {
  if (!ppv) return E_POINTER;
  *ppv = nullptr;
  if (!IsEqualCLSID(rclsid, CLSID_SrfTsfTip)) return CLASS_E_CLASSNOTAVAILABLE;

  CClassFactory* p = new (std::nothrow) CClassFactory();
  if (!p) return E_OUTOFMEMORY;

  HRESULT hr = p->QueryInterface(riid, ppv);
  p->Release();
  return hr;
}
