#include <windows.h>
#include <objbase.h>
#include <cstdio>
int wmain(int argc, wchar_t** argv) {
  if (argc != 2) return 1;
  for (int i = 0; i < 500; ++i) {
    HANDLE proceed = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    HANDLE destroyed = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    HMODULE module = LoadLibraryW(argv[1]);
    if (!module || !proceed || !destroyed) return 2;
    auto start = reinterpret_cast<BOOL (*)(HANDLE, HANDLE, BOOL)>(GetProcAddress(module, "StartWorker"));
    if (!start || !start(proceed, destroyed, i % 2)) return 3;
    // Drop the host reference while the worker is still blocked in DLL code.
    FreeLibrary(module);
    CoFreeUnusedLibraries();
    SetEvent(proceed);
    if (WaitForSingleObject(destroyed, 5000) != WAIT_OBJECT_0) return 4;
    // Wait until the worker's final module reference is released atomically
    // with thread exit; querying with UNCHANGED_REFCOUNT does not pin it.
    HMODULE remaining = nullptr;
    const ULONGLONG deadline = GetTickCount64() + 5000;
    while (GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, argv[1], &remaining)) {
      if (GetTickCount64() >= deadline) return 5;
      Sleep(1);
    }
    CloseHandle(proceed);
    CloseHandle(destroyed);
  }
  std::puts("500 DLL unload cycles passed (normal and injected exceptions)");
  return 0;
}
