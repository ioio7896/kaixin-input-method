#include "background_worker.h"
#include <stdexcept>
static LONG workers = 0;
extern "C" void SrfTip_BackgroundWorkerAddRef() { InterlockedIncrement(&workers); }
extern "C" void SrfTip_BackgroundWorkerRelease() { InterlockedDecrement(&workers); }
struct Capture {
  HANDLE destroyed;
  ~Capture() { SetEvent(destroyed); }
};
extern "C" __declspec(dllexport) BOOL StartWorker(HANDLE proceed, HANDLE destroyed, BOOL fail) {
  auto capture = std::make_shared<Capture>();
  capture->destroyed = destroyed;
  return StartDetachedBackgroundWorker([capture, proceed, fail] {
    WaitForSingleObject(proceed, INFINITE);
    if (fail) throw std::runtime_error("injected worker failure");
  });
}
