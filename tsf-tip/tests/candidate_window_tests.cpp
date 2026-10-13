#include "candidate_window.h"

#include <cstdlib>
#include <cwchar>
#include <iostream>

// Exercise the production window without writing runtime diagnostic logs.
HMODULE SrfTip_GetDllModule() { return GetModuleHandleW(nullptr); }
void SrfTsfDiagnosticLog(const wchar_t*, const wchar_t*) {}
void SrfTsfPerfLog(const wchar_t*, const wchar_t*) {}
extern "C" void SrfTip_BackgroundWorkerAddRef() {}
extern "C" void SrfTip_BackgroundWorkerRelease() {}

namespace {

void Check(bool condition, const char* message) {
  if (!condition) {
    std::cerr << "FAILED: " << message << '\n';
    std::exit(1);
  }
}

HWND CandidateHwnd() {
  HWND result = nullptr;
  EnumThreadWindows(GetCurrentThreadId(), [](HWND hwnd, LPARAM param) -> BOOL {
    wchar_t name[128] = {};
    GetClassNameW(hwnd, name, 128);
    if (wcscmp(name, L"SRF_TSF_Candidate_Window") != 0) return TRUE;
    *reinterpret_cast<HWND*>(param) = hwnd;
    return FALSE;
  }, reinterpret_cast<LPARAM>(&result));
  Check(result != nullptr, "production candidate window exists");
  return result;
}

long ClientWidth() {
  RECT client = {};
  Check(GetClientRect(CandidateHwnd(), &client) != FALSE, "read actual client width");
  return client.right - client.left;
}

void Settle() {
  // Dispatch real WM_TIMER/WM_PAINT messages, including the 96 ms shrink timer.
  const ULONGLONG deadline = GetTickCount64() + 240;
  do {
    MSG msg = {};
    while (PeekMessageW(&msg, nullptr, 0, 0, PM_REMOVE)) {
      TranslateMessage(&msg);
      DispatchMessageW(&msg);
    }
    MsgWaitForMultipleObjects(0, nullptr, FALSE, 10, QS_ALLINPUT);
  } while (GetTickCount64() < deadline);
}

const std::vector<std::wstring> longItems = {
    L"中华人民共和国", L"中华人民共和国", L"中华人民共和国"};
const std::vector<std::wstring> mediumItems = {L"开心输入法", L"开心输入法", L"开心输入法"};
const std::vector<std::wstring> shortItems = {L"开", L"心", L"好"};
const RECT anchor = {100, 100, 101, 124};

void Show(CCandidateWindow& window, const wchar_t* reading,
          const std::vector<std::wstring>& items) {
  window.Show(reading, items, {}, {}, {}, {}, 0, 1, 0, anchor);
  window.FlushPendingPaint();
}

void TestShrink() {
  CCandidateWindow window;
  SrfUIStyle style = {};
  style.candidateHorizontal = true;
  style.skinAnimationsEnabled = false;
  style.skinShadowEnabled = false;
  window.SetStyle(style);

  Show(window, L"k", shortItems);
  const long naturalShort = ClientWidth();
  window.Hide();
  Show(window, L"kaixin", longItems);
  const long original = ClientWidth();
  Check(original > naturalShort, "fixture has removable right-side width");

  // Backspace changes the reading, then a second async batch for that reading
  // arrives before the shrink timer. It must not strand the old outer width.
  Show(window, L"kaixi", mediumItems);
  Check(ClientWidth() == original, "first result briefly retains width");
  Show(window, L"kaixi", shortItems);
  Settle();
  std::cout << "same-page async refresh: " << original << " -> " << ClientWidth()
            << ", natural=" << naturalShort << '\n';
  Check(ClientWidth() == naturalShort, "same-page async result shrinks to natural width");

  window.Hide();
  Show(window, L"kaixin", longItems);
  // The reading can be updated while the previous rows remain visible.
  Show(window, L"kaixi", longItems);
  Show(window, L"kaixi", shortItems);
  Settle();
  Check(ClientWidth() == naturalShort, "retained rows replaced by shorter same-page rows shrink");

  window.Hide();
  Show(window, L"kaixin", longItems);
  Show(window, L"kaixi", mediumItems);
  Show(window, L"kaix", shortItems);
  Show(window, L"kai", shortItems);
  Settle();
  Check(ClientWidth() == naturalShort, "consecutive deletions shrink to latest reading");

  window.Hide();
  Show(window, L"kaixin", longItems);
  Show(window, L"kaixi", shortItems);
  Show(window, L"kaixi", longItems);
  Settle();
  Check(ClientWidth() == original, "growth cancels obsolete shrink target");

  Show(window, L"kaix", shortItems);
  window.Hide();
  Show(window, L"new", longItems);
  Settle();
  Check(ClientWidth() == original, "hide and reopen discard previous shrink target");
  window.Destroy();
}

}  // namespace

int main() {
  const HRESULT initialized = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  TestShrink();
  ShutdownCandidateWindowRendering();
  if (SUCCEEDED(initialized)) CoUninitialize();
  std::cout << "PASS: candidate window shrink regression tests\n";
  return 0;
}
