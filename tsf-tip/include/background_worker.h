#pragma once
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <memory>
#include <type_traits>
#include <utility>

extern "C" void SrfTip_BackgroundWorkerAddRef();
extern "C" void SrfTip_BackgroundWorkerRelease();

namespace srf_worker_detail {
template <typename Fn> struct Context {
  HMODULE module;
  Fn function;
  static DWORD WINAPI Run(void* parameter) noexcept {
    HMODULE module;
    {
      std::unique_ptr<Context> context(static_cast<Context*>(parameter));
      module = context->module;
      try { context->function(); } catch (...) {}
      // Destroy captures and callable while the module is still referenced.
    }
    SrfTip_BackgroundWorkerRelease();
    // Never return through DLL code after releasing its final module reference.
    FreeLibraryAndExitThread(module, 0);
  }
};
}

template <typename Fn>
bool StartDetachedBackgroundWorker(Fn&& function) noexcept {
  using Context = srf_worker_detail::Context<std::decay_t<Fn>>;
  HMODULE module = nullptr;
  if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                         reinterpret_cast<LPCWSTR>(&Context::Run), &module)) return false;
  SrfTip_BackgroundWorkerAddRef();
  try {
    auto context = std::make_unique<Context>(Context{module, std::forward<Fn>(function)});
    HANDLE thread = CreateThread(nullptr, 0, &Context::Run, context.get(), 0, nullptr);
    if (thread) {
      context.release();
      CloseHandle(thread);
      return true;
    }
  } catch (...) {}
  SrfTip_BackgroundWorkerRelease();
  FreeLibrary(module);
  return false;
}
