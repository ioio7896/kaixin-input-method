#include <windows.h>

#include <cstdio>
#include <string>

int SrfTip_TestIdleRecovery(DWORD idleMs, std::wstring* detail);

extern "C" void SrfTip_BackgroundWorkerAddRef() {
}
extern "C" void SrfTip_BackgroundWorkerRelease() {
}

int wmain() {
  std::wstring detail;
  const int result = SrfTip_TestIdleRecovery(305000, &detail);
  if (result) {
    std::fwprintf(stderr, L"idle recovery failed (%d): %ls\n", result, detail.c_str());
    return result;
  }
  std::puts("{\"idle_disconnect\":true,\"first_lookup\":true,\"same_engine\":true}");
  return 0;
}
