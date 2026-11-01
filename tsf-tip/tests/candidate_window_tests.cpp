#include "candidate_window.h"
#include "ime_config.h"
#include <UIAutomation.h>
#include <wrl/client.h>
#include <msctf.h>
#include <ctffunc.h>

#include <cstdlib>
#include <cwchar>
#include <iostream>
#include <algorithm>
#include <cmath>
#include <fstream>

// Test bundled skins without starting the user's live configuration watcher.
void LoadSkinFileFromConfigDir(SrfConfig&, const std::filesystem::path&);

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

#include "../src/candidate_window_parts/candidate_window_theme.ipp"

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

void TestClipboardTextColors() {
  SrfUIStyle style = {};
  style.skinLoaded = true;
  style.skinWindowBg = RGB(25, 27, 30);
  style.skinText = RGB(235, 240, 245);
  style.skinMutedText = RGB(190, 200, 210);
  // A skin can have light interaction cards on a dark window. Those cards
  // are not painted for unselected VVU rows and must not turn their text black.
  style.skinItemBg = RGB(255, 255, 255);
  style.skinHoverBg = RGB(240, 245, 250);
  style.skinPressedBg = RGB(220, 230, 240);
  const CandidateColors colors = ResolveColors(style);
  Check(colors.text == style.skinText, "VVU preserves readable unselected skin text");
  Check(colors.mutedText == style.skinMutedText, "VVU preserves readable detail text");
  Check(ContrastRatio(colors.text, colors.windowBg) >= 4.2,
        "VVU non-first rows remain readable on actual dark background");
  for (COLORREF fill : {colors.hoverBg, colors.pressedBg, RGB(30, 60, 50)}) {
    Check(ContrastRatio(CandidateRowTextColor(colors.text, colors, true, fill, 4.2), fill) >= 4.2,
          "VVU selected, hovered and pressed text follows actual row fill");
  }
  style.skinWindowBg = RGB(250, 250, 250);
  style.skinText = RGB(30, 40, 50);
  style.skinPressedBg = RGB(15, 20, 25);
  Check(ResolveColors(style).text == style.skinText,
        "VVU light theme preserves readable skin text despite dark pressed fill");
  style.skinWindowBg = RGB(20, 20, 20);
  style.skinWindowBgTo = RGB(60, 60, 60);
  style.skinText = RGB(0, 0, 0);
  const CandidateColors gradient = ResolveColors(style);
  Check(ContrastRatio(gradient.text, gradient.windowBg) >= 4.2 &&
            ContrastRatio(gradient.text, gradient.windowBgTo) >= 4.2,
        "VVU corrects unreadable text against both gradient endpoints");
}

std::filesystem::path SkinsRoot() {
  return std::filesystem::u8path(__FILE__).parent_path().parent_path().parent_path() / "skins";
}

void TestSkinContrast() {
  unsigned count = 0;
  Check(std::filesystem::is_directory(SkinsRoot()), "source skin directory exists");
  for (const auto& entry : std::filesystem::directory_iterator(SkinsRoot())) {
    const auto theme = entry.path() / "theme.json";
    if (!std::filesystem::exists(theme)) continue;
    SrfConfig config;
    config.style.candidateSkinFile = theme.wstring();
    LoadSkinFileFromConfigDir(config, SkinsRoot().parent_path());
    Check(config.style.skinLoaded, "bundled skin loads through production parser");
    const auto colors = ResolveColors(config.style);
    for (COLORREF target : {colors.selectedBg, colors.hoverBg, colors.pressedBg}) {
      for (int step = 0; step <= 20; ++step) {
        const float progress = step / 20.0f;
        const COLORREF fill = AlphaBlendColor(target, colors.windowBg, progress);
        const COLORREF text = AlphaBlendColor(colors.selectedText, colors.text, progress);
        const COLORREF muted = AlphaBlendColor(colors.selectedMutedText, colors.mutedText, progress);
        Check(ContrastRatio(CandidateRowTextColor(text, colors, true, fill, 4.5), fill) >= 4.5,
              "skin body text readable throughout state transition");
        Check(ContrastRatio(CandidateRowTextColor(muted, colors, true, fill, 4.5), fill) >= 4.5,
              "skin metadata readable throughout state transition");
      }
    }
    Check(ContrastRatio(colors.chipText, colors.chipBg) >= 4.5 &&
              ContrastRatio(colors.chipActiveText, colors.chipActiveBg) >= 4.5,
          "active and inactive chips use their own backgrounds");
    ++count;
  }
  Check(count >= 35, "original skins and twenty new skins are present");
  std::cout << "  checked " << count << " skins and all transition frames\n";
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

void RenderSkinSamples() {
  const auto destination = SkinsRoot().parent_path() / "dist" / "candidate-design-preview";
  std::filesystem::create_directories(destination);
  for (const auto& entry : std::filesystem::directory_iterator(SkinsRoot())) {
    const auto theme = entry.path() / "theme.json";
    if (!std::filesystem::exists(theme)) continue;
    for (bool clipboard : {false, true}) {
      SrfConfig config;
      config.style.candidateSkinFile = theme.wstring();
      LoadSkinFileFromConfigDir(config, SkinsRoot().parent_path());
      config.style.candidateHorizontal = !clipboard;
      config.style.candidateReduceMotion = true;
      CCandidateWindow window;
      window.SetStyle(config.style);
      const std::vector<std::wstring> items = clipboard
          ? std::vector<std::wstring>{L"\u8bbe\u8ba1\u8981\u70b9\uff1a\u5c42\u7ea7\u6e05\u6670\uff0c\u9009\u4e2d\u9192\u76ee\u3002\n\u6b63\u6587\u4e0e\u8bf4\u660e\u5404\u6709\u8282\u594f\u3002\n\u66f4\u591a\u5185\u5bb9\u5728\u9009\u4e2d\u540e\u5c55\u5f00\u3002",
              L"\u660e\u5929\u4e0b\u5348\u4e09\u70b9\u8ba8\u8bba\u65b0\u7248\u5019\u9009\u680f\u3002\n\u8bf7\u5e26\u4e0a\u4f60\u559c\u6b22\u7684\u5b57\u4f53\u548c\u76ae\u80a4\u3002\n\u672a\u9009\u4e2d\u65f6\u4e0d\u663e\u793a\u7b2c\u4e09\u884c\u3002",
              L"https://example.com/design"}
          : std::vector<std::wstring>{L"\u5f00\u5fc3", L"\u5f00\u65b0", L"\u5f00\u4fe1", L"\u5f00\u5fc3\u8f93\u5165\u6cd5"};
      std::vector<std::wstring> comments(items.size(), clipboard ? L"\u8bb0\u4e8b\u672c \u00b7 14:30\t\u6587\u672c" : L"");
      std::vector<std::wstring> labels;
      for (size_t i = 0; i < items.size(); ++i) labels.push_back(std::to_wstring(i + 1));
      std::vector<bool> pins(items.size(), false);
      pins[0] = clipboard;
      window.Show(clipboard ? L"\u5feb\u6377\u526a\u8d34\u677f" : L"kaixin", items, comments, labels,
                  pins, std::vector<bool>(items.size(), clipboard), 0, 2, 0, anchor);
      window.FlushPendingPaint();
      RECT rect = {};
      GetClientRect(CandidateHwnd(), &rect);
      BITMAPINFO info = {};
      info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
      info.bmiHeader.biWidth = rect.right;
      info.bmiHeader.biHeight = -rect.bottom;
      info.bmiHeader.biPlanes = 1;
      info.bmiHeader.biBitCount = 32;
      info.bmiHeader.biCompression = BI_RGB;
      const DWORD size = rect.right * rect.bottom * 4;
      HDC dc = CreateCompatibleDC(nullptr);
      void* bits = nullptr;
      HBITMAP bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &bits, nullptr, 0);
      Check(dc && bitmap && bits, "allocate skin preview bitmap");
      HGDIOBJ old = SelectObject(dc, bitmap);
      SendMessageW(CandidateHwnd(), WM_PRINTCLIENT, reinterpret_cast<WPARAM>(dc), PRF_CLIENT);
      GdiFlush();
      BITMAPFILEHEADER header = {};
      header.bfType = 0x4d42;
      header.bfOffBits = sizeof(header) + sizeof(BITMAPINFOHEADER);
      header.bfSize = header.bfOffBits + size;
      const auto path = destination / (entry.path().filename().wstring() + (clipboard ? L"-clipboard.bmp" : L"-horizontal.bmp"));
      std::ofstream output(path, std::ios::binary);
      output.write(reinterpret_cast<const char*>(&header), sizeof(header));
      output.write(reinterpret_cast<const char*>(&info.bmiHeader), sizeof(BITMAPINFOHEADER));
      output.write(reinterpret_cast<const char*>(bits), size);
      Check(output.good(), "save native skin preview");
      SelectObject(dc, old);
      DeleteObject(bitmap);
      DeleteDC(dc);
      window.Destroy();
    }
  }
  std::cout << "  native skin samples rendered\n";
}

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

int main(int argc, char** argv) {
  std::cout << std::unitbuf;
  const HRESULT initialized = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
  std::cout << "test: clipboard text colors\n";
  TestClipboardTextColors();
  std::cout << "test: bundled skin contrast\n";
  try { TestSkinContrast(); }
  catch (const std::exception& error) { std::cerr << "skin test exception: " << error.what() << '\n'; return 1; }
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
  if (argc > 1 && std::string(argv[1]) == "--render-skins") RenderSkinSamples();
  std::cout << "test: shutdown\n";
  ShutdownCandidateWindowRendering();
  if (SUCCEEDED(initialized)) CoUninitialize();
  std::cout << "PASS: candidate window shrink regression tests\n";
  return 0;
}
