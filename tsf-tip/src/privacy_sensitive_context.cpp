#include "privacy_sensitive_context.h"

#include <cwchar>
#include <unordered_set>

namespace {

bool WildcardMatchNoCase(const std::wstring& pattern, const std::wstring& text) {
  size_t p = 0;
  size_t t = 0;
  size_t star = std::wstring::npos;
  size_t retry = 0;
  std::wstring pat = pattern;
  std::wstring value = text;
  for (wchar_t& ch : pat) ch = static_cast<wchar_t>(towlower(ch));
  for (wchar_t& ch : value) ch = static_cast<wchar_t>(towlower(ch));
  while (t < value.size()) {
    if (p < pat.size() && (pat[p] == L'?' || pat[p] == value[t])) {
      ++p;
      ++t;
    } else if (p < pat.size() && pat[p] == L'*') {
      star = p++;
      retry = t;
    } else if (star != std::wstring::npos) {
      p = star + 1;
      t = ++retry;
    } else {
      return false;
    }
  }
  while (p < pat.size() && pat[p] == L'*') ++p;
  return p == pat.size();
}

std::wstring WindowClassName(HWND hwnd) {
  wchar_t name[128] = {};
  if (!hwnd || GetClassNameW(hwnd, name, static_cast<int>(_countof(name))) <= 0) return {};
  return name;
}

std::wstring BaseName(std::wstring path) {
  const size_t slash = path.find_last_of(L"\\/");
  if (slash != std::wstring::npos) path = path.substr(slash + 1);
  return path;
}

bool IsBuiltinSensitiveProcessName(const std::wstring& appName) {
  if (appName.empty()) return false;
  const std::wstring baseName = BaseName(appName);
  const wchar_t* patterns[] = {
      L"credentialui*.exe", L"credwiz.exe",   L"keepass*.exe",
      L"1password*.exe",    L"bitwarden*.exe", L"lastpass*.exe",
      L"dashlane*.exe",     L"enpass*.exe",    L"authy*.exe",
  };
  for (const wchar_t* pattern : patterns) {
    if (WildcardMatchNoCase(pattern, baseName)) return true;
  }
  return false;
}

HWND CurrentSensitiveFocusWindow() {
  GUITHREADINFO info = {};
  info.cbSize = sizeof(info);
  if (GetGUIThreadInfo(0, &info)) {
    if (info.hwndFocus) return info.hwndFocus;
    if (info.hwndCaret) return info.hwndCaret;
  }
  HWND focus = GetFocus();
  if (focus) return focus;
  return GetForegroundWindow();
}

bool WindowOrParentHasPasswordStyle(HWND hwnd) {
  std::unordered_set<HWND> seen;
  for (HWND current = hwnd; current && IsWindow(current) && seen.insert(current).second;
       current = GetParent(current)) {
    const LONG_PTR style = GetWindowLongPtrW(current, GWL_STYLE);
    if ((style & ES_PASSWORD) != 0) return true;

    const std::wstring className = WindowClassName(current);
    if (!className.empty() &&
        (WildcardMatchNoCase(L"*password*", className) ||
         WildcardMatchNoCase(L"*credential*", className))) {
      return true;
    }
  }
  return false;
}

}  // namespace

bool SrfIsSensitiveInputContext(const std::wstring& appName) {
  if (IsBuiltinSensitiveProcessName(appName)) return true;
  return WindowOrParentHasPasswordStyle(CurrentSensitiveFocusWindow());
}
