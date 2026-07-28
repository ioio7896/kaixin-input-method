#include <windows.h>
#include <objbase.h>

#include "candidate_window.h"
#include "guids.h"

LONG g_cSrfTipObjects = 0;
static LONG g_cServerLocks = 0;
static LONG g_cBackgroundWorkers = 0;
static HMODULE g_hModule = nullptr;

/// 供窗口类注册等场景使用，避免 GetModuleHandleW(nullptr) 返回宿主 EXE 句柄。
HMODULE SrfTip_GetDllModule() { return g_hModule; }

HRESULT SrfTip_DllGetClassObject(REFCLSID rclsid, REFIID riid, void** ppv);

HRESULT SrfTip_RegisterServer(HMODULE hModule);
HRESULT SrfTip_UnregisterServer();
HRESULT SrfTip_RegisterServerEx(HMODULE hModule, BOOL machineScope);
HRESULT SrfTip_UnregisterServerEx(BOOL machineScope);

BOOL APIENTRY DllMain(HMODULE hModule, DWORD dwReason, LPVOID /*lpReserved*/) {
  switch (dwReason) {
    case DLL_PROCESS_ATTACH:
      g_hModule = hModule;
      DisableThreadLibraryCalls(hModule);
      break;
    case DLL_PROCESS_DETACH:
      ShutdownCandidateWindowRendering();
      break;
  }
  return TRUE;
}

STDAPI DllGetClassObject(REFCLSID rclsid, REFIID riid, LPVOID* ppv) {
  return SrfTip_DllGetClassObject(rclsid, riid, ppv);
}

STDAPI DllCanUnloadNow(void) {
  if (g_cSrfTipObjects > 0 || g_cServerLocks > 0 || g_cBackgroundWorkers > 0) return S_FALSE;
  return S_OK;
}

STDAPI DllRegisterServer(void) {
  HMODULE mod = g_hModule;
  if (!mod) {
    (void)GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                             reinterpret_cast<LPCWSTR>(&DllRegisterServer), &mod);
  }
  return SrfTip_RegisterServer(mod);
}

STDAPI DllUnregisterServer(void) { return SrfTip_UnregisterServer(); }

STDAPI DllInstall(BOOL bInstall, LPCWSTR pszCmdLine) {
  HMODULE mod = g_hModule;
  if (!mod) {
    (void)GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                             reinterpret_cast<LPCWSTR>(&DllInstall), &mod);
  }
  BOOL machine = FALSE;
  if (pszCmdLine) {
    if (wcsstr(pszCmdLine, L"machine") != nullptr || wcsstr(pszCmdLine, L"Machine") != nullptr)
      machine = TRUE;
  }
  if (bInstall) return SrfTip_RegisterServerEx(mod, machine);
  return SrfTip_UnregisterServerEx(machine);
}

extern "C" void SrfTip_LockServer(BOOL lock) {
  if (lock)
    InterlockedIncrement(&g_cServerLocks);
  else
    InterlockedDecrement(&g_cServerLocks);
}

extern "C" void SrfTip_BackgroundWorkerAddRef() {
  InterlockedIncrement(&g_cBackgroundWorkers);
}

extern "C" void SrfTip_BackgroundWorkerRelease() {
  InterlockedDecrement(&g_cBackgroundWorkers);
}
