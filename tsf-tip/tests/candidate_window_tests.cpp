#include "candidate_window.h"
#include <UIAutomation.h>
#include <wrl/client.h>
#include <msctf.h>
#include <ctffunc.h>

#include <cstdlib>
#include <cwchar>
#include <iostream>

// Exercise the production window without writing runtime diagnostic logs.
HMODULE SrfTip_GetDllModule() {
  return GetModuleHandleW(nullptr);
}
void SrfTsfDiagnosticLog(const wchar_t*, const wchar_t*) {
}
void SrfTsfPerfLog(const wchar_t*, const wchar_t*) {
}
extern "C" void SrfTip_BackgroundWorkerAddRef() {
}
extern "C" void SrfTip_BackgroundWorkerRelease() {
}

namespace {

using Microsoft::WRL::ComPtr;
unsigned imeShows = 0, imeHides = 0, imeMoves = 0;
void CALLBACK OnImeEvent(HWINEVENTHOOK, DWORD event, HWND, LONG, LONG, DWORD, DWORD) {
  if (event == EVENT_OBJECT_IME_SHOW) ++imeShows;
  if (event == EVENT_OBJECT_IME_HIDE) ++imeHides;
  if (event == EVENT_OBJECT_IME_CHANGE) ++imeMoves;
}

void Check(bool condition, const char* message) {
  if (!condition) {
    std::cerr << "FAILED: " << message << '\n';
    std::exit(1);
  }
}

HWND CandidateHwnd() {
  HWND result = nullptr;
  EnumThreadWindows(
      GetCurrentThreadId(),
      [](HWND hwnd, LPARAM param) -> BOOL {
        wchar_t name[128] = {};
        GetClassNameW(hwnd, name, 128);
        if (wcscmp(name, L"SRF_TSF_Candidate_Window") != 0) return TRUE;
        *reinterpret_cast<HWND*>(param) = hwnd;
        return FALSE;
      },
      reinterpret_cast<LPARAM>(&result));
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

const std::vector<std::wstring> longItems = {L"中华人民共和国", L"中华人民共和国",
                                             L"中华人民共和国"};
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

void TestInputPanePlacement() {
  const RECT work = {-1920, 0, 0, 1080};
  const RECT docked = {-1920, 700, 0, 1080};
  const RECT original = {-1200, 740, -850, 940};
  RECT adjusted = SrfAvoidInputPane(original, docked, work, 6);
  RECT overlap = {};
  Check(!IntersectRect(&overlap, &adjusted, &docked), "candidate avoids docked keyboard");
  Check(adjusted.right - adjusted.left == 350 && adjusted.bottom - adjusted.top == 200,
        "keyboard avoidance preserves candidate dimensions");
  const RECT floating = {-1000, 300, -400, 650};
  adjusted = SrfAvoidInputPane({-900, 400, -600, 550}, floating, work, 6);
  Check(!IntersectRect(&overlap, &adjusted, &floating), "candidate avoids floating keyboard");
  Check(adjusted.left >= work.left && adjusted.right <= work.right && adjusted.top >= work.top &&
            adjusted.bottom <= work.bottom,
        "keyboard avoidance stays on monitor");
  adjusted = SrfAvoidInputPane(original, {}, work, 6);
  Check(EqualRect(&adjusted, &original), "hidden keyboard keeps normal anchor");
}

void TestOwnerAndEvents() {
  HWND owner = CreateWindowExW(0, L"STATIC", L"owner", WS_POPUP, 0, 0, 10, 10, nullptr, nullptr,
                               GetModuleHandleW(nullptr), nullptr);
  Check(owner != nullptr, "create host owner");
  // Filter by this test's thread to avoid observing another application's IME.
  HWINEVENTHOOK hook =
      SetWinEventHook(EVENT_OBJECT_IME_SHOW, EVENT_OBJECT_IME_CHANGE, nullptr, OnImeEvent,
                      GetCurrentProcessId(), GetCurrentThreadId(), WINEVENT_OUTOFCONTEXT);
  Check(hook != nullptr, "subscribe to IME window notifications");
  CCandidateWindow window;
  window.SetOwnerWindow(owner);
  Show(window, L"owner", shortItems);
  Check(GetWindow(CandidateHwnd(), GW_OWNER) == owner, "candidate belongs to focused host");
  Settle();
  RECT rect = {};
  GetWindowRect(CandidateHwnd(), &rect);
  SetWindowPos(CandidateHwnd(), nullptr, rect.left + 20, rect.top, 0, 0,
               SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
  Settle();
  window.Hide();
  Settle();
  Check(imeShows > 0 && imeHides > 0 && imeMoves > 0, "IME show/hide/change events emitted");
  Show(window, L"closing host", shortItems);
  DestroyWindow(owner);
  Check(!window.IsVisible(), "closing owner destroys its candidate window");
  window.SetOwnerWindow(nullptr);
  Show(window, L"new host", shortItems);
  Check(window.IsVisible(), "candidate window can be recreated after owner destruction");
  window.Destroy();
  UnhookWinEvent(hook);
}

void TestAccessibilityLifetime() {
  HWND hwnd = CreateWindowExW(WS_EX_NOACTIVATE, L"STATIC", L"UIA", WS_POPUP, 20, 20, 300, 120,
                              nullptr, nullptr, GetModuleHandleW(nullptr), nullptr);
  Check(hwnd != nullptr, "create accessible window");
  ShowWindow(hwnd, SW_SHOWNOACTIVATE);
  SrfCandidateAccessibility accessibility;
  const std::vector<std::wstring> items = {L"你好", L"拟好"};
  const std::vector<RECT> rectangles = {{0, 0, 300, 50}, {0, 50, 300, 100}};
  accessibility.Update(hwnd, L"nihao", items, {L"常用词", L""}, rectangles, 0, 0, true);
  ComPtr<IRawElementProviderSimple> root;
  Check(SUCCEEDED(accessibility.ProviderForWindow(hwnd, &root)), "obtain actual UIA provider");
  VARIANT property = {};
  Check(SUCCEEDED(root->GetPropertyValue(UIA_AutomationIdPropertyId, &property)) &&
            property.vt == VT_BSTR && wcscmp(property.bstrVal, L"IME_Candidate_Window") == 0,
        "standard IME automation ID");
  VariantClear(&property);
  ComPtr<IRawElementProviderFragment> fragment;
  Check(SUCCEEDED(root.As(&fragment)), "root exposes fragment tree");
  ComPtr<IRawElementProviderFragment> child;
  Check(SUCCEEDED(fragment->Navigate(NavigateDirection_FirstChild, &child)) && child,
        "navigate to first candidate");
  ComPtr<IRawElementProviderSimple> item;
  Check(SUCCEEDED(child.As(&item)), "candidate exposes properties");
  Check(SUCCEEDED(item->GetPropertyValue(UIA_NamePropertyId, &property)) &&
            property.vt == VT_BSTR && wcscmp(property.bstrVal, L"你好") == 0,
        "Narrator receives full candidate text");
  VariantClear(&property);
  ComPtr<ISelectionItemProvider> selected;
  Check(SUCCEEDED(child.As(&selected)), "candidate selection pattern");
  BOOL isSelected = FALSE;
  Check(SUCCEEDED(selected->get_IsSelected(&isSelected)) && isSelected, "first candidate selected");
  accessibility.Update(hwnd, L"nihao", items, {}, rectangles, 0, 1, true);
  Check(SUCCEEDED(selected->get_IsSelected(&isSelected)) && !isSelected,
        "selection follows actual highlight");
  accessibility.Update(hwnd, L"next", {L"新候选"}, {}, rectangles, 0, 0, true);
  Check(selected->Select() == UIA_E_ELEMENTNOTAVAILABLE, "old page cannot select a new candidate");
  accessibility.Hide();
  accessibility.Destroy();
  Check(root->GetPropertyValue(UIA_NamePropertyId, &property) == UIA_E_ELEMENTNOTAVAILABLE,
        "retained root safely rejects access after window destruction");
  accessibility.Update(hwnd, L"reopened", items, {}, rectangles, 0, 0, true);
  Check(root->GetPropertyValue(UIA_NamePropertyId, &property) == UIA_E_ELEMENTNOTAVAILABLE,
        "old root stays invalid after window recreation");
  accessibility.Destroy();
  DestroyWindow(hwnd);
}

void TestTouchKeyboardContract() {
  const auto dataRoot = std::filesystem::temp_directory_path() /
                        (L"KaixinTouchContract-" + std::to_wstring(GetCurrentProcessId()));
  std::filesystem::create_directories(dataRoot);
  Check(SetEnvironmentVariableW(L"LOCALAPPDATA", dataRoot.c_str()) != FALSE,
        "isolate TIP configuration from installed user data");
  wchar_t path[MAX_PATH] = {};
  Check(GetModuleFileNameW(nullptr, path, ARRAYSIZE(path)) != 0, "locate test executable");
  const auto tipPath = std::filesystem::path(path).parent_path() / L"srf_tsf_tip.dll";
  HMODULE tip = LoadLibraryW(tipPath.c_str());
  Check(tip != nullptr, "load matching-architecture TIP without registering it");
  std::cout << "  TIP loaded\n";
  using FactoryFunction = HRESULT(STDAPICALLTYPE*)(REFCLSID, REFIID, void**);
  auto factoryFunction =
      reinterpret_cast<FactoryFunction>(GetProcAddress(tip, "DllGetClassObject"));
  Check(factoryFunction != nullptr, "TIP exports class factory");
  const CLSID clsid = {
      0xe5a91c40, 0x7b2d, 0x4f8a, {0x9c, 0x11, 0x8f, 0x3e, 0x6d, 0x2a, 0x1b, 0x00}};
  {
    ComPtr<IClassFactory> factory;
    Check(SUCCEEDED(factoryFunction(clsid, IID_IClassFactory,
                                    reinterpret_cast<void**>(factory.GetAddressOf()))),
          "obtain TIP factory");
    std::cout << "  factory acquired\n";
    ComPtr<ITfFunctionProvider> functions;
    Check(SUCCEEDED(factory->CreateInstance(nullptr, IID_ITfFunctionProvider,
                                            reinterpret_cast<void**>(functions.GetAddressOf()))),
          "TIP exposes function provider");
    std::cout << "  function provider created\n";
    ComPtr<IUnknown> unknown;
    Check(SUCCEEDED(functions->GetFunction(GUID_NULL, IID_ITfFnGetPreferredTouchKeyboardLayout,
                                           &unknown)),
          "system can request preferred keyboard layout");
    ComPtr<ITfFnGetPreferredTouchKeyboardLayout> layout;
    Check(SUCCEEDED(unknown.As(&layout)), "keyboard layout interface is queryable");
    TKBLayoutType type = TKBLT_UNDEFINED;
    WORD id = 0;
    Check(SUCCEEDED(layout->GetLayout(&type, &id)) && type == TKBLT_OPTIMIZED && id == 0x0804,
          "TIP requests simplified Chinese pinyin touch layout");
    Check(layout->GetLayout(nullptr, &id) == E_POINTER &&
              layout->GetLayout(&type, nullptr) == E_POINTER,
          "touch keyboard interface validates output pointers");
    std::cout << "  layout contract passed\n";
  }
  std::cout << "  COM interfaces released\n";
  FreeLibrary(tip);
}

}  // namespace

int main() {
  std::cout << std::unitbuf;
  const HRESULT initialized = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  std::cout << "test: shrink\n";
  TestShrink();
  std::cout << "test: keyboard placement\n";
  TestInputPanePlacement();
  std::cout << "test: owner lifecycle\n";
  TestOwnerAndEvents();
  std::cout << "test: accessibility lifetime\n";
  TestAccessibilityLifetime();
  std::cout << "test: touch keyboard COM contract\n";
  TestTouchKeyboardContract();
  std::cout << "test: shutdown\n";
  ShutdownCandidateWindowRendering();
  if (SUCCEEDED(initialized)) CoUninitialize();
  std::cout << "PASS: candidate window shrink regression tests\n";
  return 0;
}
