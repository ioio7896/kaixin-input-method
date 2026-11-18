// Runs against the staged DLL, without COM registration or a TSF host.
#include <windows.h>
#include <cstdio>
#include <cwchar>

int wmain(int argc, wchar_t** argv) {
  if (argc != 3) return 2;
  HMODULE module = LoadLibraryExW(argv[2], nullptr,
                                  LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
  if (!module) {
    std::fprintf(stderr, "LoadLibrary failed: %lu\n", GetLastError());
    return 3;
  }
  auto info = reinterpret_cast<const char*(__cdecl*)()>(GetProcAddress(module, "KxGetBuildInfo"));
  auto probe =
      reinterpret_cast<int(__cdecl*)(wchar_t*, unsigned)>(GetProcAddress(module, "KxReleaseProbe"));
  int result = 4;
  if (info && probe) {
    if (std::wcscmp(argv[1], L"--metadata") == 0) {
      std::puts(info());
      result = 0;
    } else if (std::wcscmp(argv[1], L"--probe") == 0) {
      wchar_t error[2048] = {};
      result = probe(error, 2048);
      if (result == 0)
        std::puts("{\"handshake\":true,\"input\":\"nihao\",\"matched\":true}");
      else
        std::fwprintf(stderr, L"staged TSF probe failed (%d): %ls\n", result, error);
    }
  } else {
    std::fputs("missing release metadata/probe exports; rebuild both TSF architectures\n", stderr);
  }
  FreeLibrary(module);
  return result;
}
