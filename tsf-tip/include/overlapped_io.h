#pragma once

#include <windows.h>

#include <memory>
#include <new>
#include <vector>

// Each operation owns its buffer and a duplicate pipe/directory handle. A timed
// out caller can leave while cancellation finishes without releasing kernel
// referenced memory. The cleanup callback keeps this module loaded until return.
class SrfOverlappedIo {
 public:
  using Ptr = std::unique_ptr<SrfOverlappedIo, void (*)(SrfOverlappedIo*)>;

  static Ptr Create(HANDLE source, size_t bytes) {
    Ptr operation(new (std::nothrow) SrfOverlappedIo(), Dispose);
    if (!operation) {
      SetLastError(ERROR_NOT_ENOUGH_MEMORY);
      return operation;
    }
    if (!DuplicateHandle(GetCurrentProcess(), source, GetCurrentProcess(), &operation->handle,
                         0, FALSE, DUPLICATE_SAME_ACCESS)) {
      return Ptr(nullptr, Dispose);
    }
    operation->overlapped.hEvent = CreateEventW(nullptr, TRUE, FALSE, nullptr);
    if (!operation->overlapped.hEvent) return Ptr(nullptr, Dispose);
    try {
      operation->buffer.resize(bytes);
    } catch (const std::bad_alloc&) {
      SetLastError(ERROR_NOT_ENOUGH_MEMORY);
      return Ptr(nullptr, Dispose);
    }
    return operation;
  }

  bool Await(DWORD timeoutMs, DWORD* transferred) {
    const DWORD wait = WaitForSingleObject(overlapped.hEvent, timeoutMs);
    if (wait == WAIT_OBJECT_0) {
      const BOOL completed = GetOverlappedResult(handle, &overlapped, transferred, FALSE);
      const DWORD error = completed ? ERROR_SUCCESS : GetLastError();
      pending = !completed && error == ERROR_IO_INCOMPLETE;
      SetLastError(error);
      return completed != FALSE;
    }
    if (wait == WAIT_TIMEOUT) SetLastError(ERROR_TIMEOUT);
    return false;
  }

  HANDLE handle = nullptr;
  OVERLAPPED overlapped = {};
  std::vector<BYTE> buffer;
  bool pending = false;

 private:
  SrfOverlappedIo() = default;
  ~SrfOverlappedIo() {
    if (overlapped.hEvent) CloseHandle(overlapped.hEvent);
    if (handle) CloseHandle(handle);
  }

  void Drain() {
    DWORD transferred = 0;
    (void)GetOverlappedResult(handle, &overlapped, &transferred, TRUE);
    pending = false;
  }

  static void CALLBACK Cleanup(PTP_CALLBACK_INSTANCE instance, void* context, PTP_WORK work) {
    auto* operation = static_cast<SrfOverlappedIo*>(context);
    const HMODULE module = operation->cleanupModule;
    operation->Drain();
    delete operation;
    CloseThreadpoolWork(work);
    FreeLibraryWhenCallbackReturns(instance, module);
  }

  static void Dispose(SrfOverlappedIo* operation) {
    if (!operation) return;
    const DWORD savedError = GetLastError();
    if (operation->pending) {
      (void)CancelIoEx(operation->handle, &operation->overlapped);
      DWORD transferred = 0;
      if (!operation->Await(200, &transferred) && operation->pending) {
        HMODULE module = nullptr;
        if (GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                              reinterpret_cast<LPCWSTR>(&Cleanup), &module)) {
          operation->cleanupModule = module;
          PTP_WORK work = CreateThreadpoolWork(Cleanup, operation, nullptr);
          if (work) {
            SubmitThreadpoolWork(work);
            SetLastError(savedError);
            return;
          }
          // Allocation failure: retain safety by draining on the caller before
          // releasing its temporary module reference.
          operation->Drain();
          FreeLibrary(module);
        } else {
          operation->Drain();
        }
      }
    }
    delete operation;
    SetLastError(savedError);
  }

  HMODULE cleanupModule = nullptr;
};
