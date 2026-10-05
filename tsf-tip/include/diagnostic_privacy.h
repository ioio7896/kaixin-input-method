#pragma once

#include <string>
#include <string_view>

inline std::wstring TrimDiagnosticToken(std::wstring token) {
  while (!token.empty() && (token.front() == L',' || token.front() == L';' || token.front() == L' ')) {
    token.erase(token.begin());
  }
  while (!token.empty() && (token.back() == L',' || token.back() == L';' || token.back() == L' ')) {
    token.pop_back();
  }
  return token;
}

inline std::wstring LowerAsciiForDiagnostics(std::wstring value) {
  for (wchar_t& ch : value) {
    if (ch >= L'A' && ch <= L'Z') ch = static_cast<wchar_t>(ch - L'A' + L'a');
  }
  return value;
}

inline bool IsSafeDiagnosticKey(const std::wstring& key) {
  const std::wstring lower = LowerAsciiForDiagnostics(key);
  static constexpr const wchar_t* keys[] = {
      L"anchor", L"anchored", L"appcontainer", L"async", L"candidateempty", L"compat",
      L"compathide", L"composing", L"contextempty", L"count", L"currentserial", L"cursor",
      L"delayms", L"direct", L"dwflags", L"elapsed", L"elapsed_ms", L"engine", L"fallback",
      L"flags", L"full", L"generation", L"grace_ms_left", L"hasanchor", L"hasrect", L"hr",
      L"immersive", L"integrity", L"items", L"lookup_status", L"offset", L"page", L"pages",
      L"partial", L"pid", L"prefix_placeholder", L"reason", L"request_id", L"refreshcandidates",
      L"retained", L"retry", L"raw_fallback_suppressed", L"secure", L"selected",
      L"selectedinpage", L"sensitive", L"show", L"shown", L"showwindow", L"stage", L"status",
      L"state", L"tid", L"total", L"uielement", L"uielementid", L"uiless", L"uilessmode",
      L"visible", L"reading_units", L"current_reading_units", L"result_reading_units"};
  for (const wchar_t* safe : keys) {
    if (lower == safe) return true;
  }
  return false;
}

inline bool IsNumericDiagnosticValue(std::wstring_view value) {
  if (value.empty() || value.size() > 32) return false;
  size_t pos = 0;
  if (value[pos] == L'-' || value[pos] == L'+') ++pos;
  if (pos == value.size()) return false;
  if (value.size() > pos + 2 && value[pos] == L'0' &&
      (value[pos + 1] == L'x' || value[pos + 1] == L'X')) {
    for (pos += 2; pos < value.size(); ++pos) {
      const wchar_t ch = value[pos];
      if (!((ch >= L'0' && ch <= L'9') || (ch >= L'a' && ch <= L'f') ||
            (ch >= L'A' && ch <= L'F'))) return false;
    }
    return true;
  }
  bool digit = false;
  bool dot = false;
  for (; pos < value.size(); ++pos) {
    const wchar_t ch = value[pos];
    if (ch >= L'0' && ch <= L'9') digit = true;
    else if (ch == L'.' && !dot) dot = true;
    else return false;
  }
  return digit;
}

inline bool IsSafeDiagnosticValue(const std::wstring& key, const std::wstring& value) {
  if (IsNumericDiagnosticValue(value)) return true;
  const std::wstring lowerKey = LowerAsciiForDiagnostics(key);
  const std::wstring lowerValue = LowerAsciiForDiagnostics(value);
  // Text values are closed enums. Never allow arbitrary strings merely because
  // their field name was approved; numeric fields accept no text at all.
  if (lowerKey == L"status" || lowerKey == L"lookup_status" || lowerKey == L"state" ||
      lowerKey == L"engine") {
    static constexpr const wchar_t* values[] = {
        L"ok", L"failed", L"skipped", L"slow", L"empty", L"busy", L"pending", L"ready",
        L"loading", L"unavailable", L"cancelled", L"superseded", L"timeout", L"stale",
        L"disconnected", L"not-connected", L"none", L"partial", L"complete"};
    for (const wchar_t* safe : values) if (lowerValue == safe) return true;
  } else if (lowerKey == L"reason") {
    static constexpr const wchar_t* values[] = {
        L"privacy_disabled", L"serial", L"reading", L"empty_current", L"foreground-changed",
        L"resolve_failed", L"resolve_failed_fallback_inline", L"syllable_bounds_unavailable",
        L"clipboard_transport", L"clipboard_changed_during_paste", L"single-char-continuation",
        L"candidate-interaction", L"async-drop-stale", L"async-drop-pending",
        L"result-reading-mismatch", L"async-drop-focus", L"focus", L"cancelled", L"timeout"};
    for (const wchar_t* safe : values) if (lowerValue == safe) return true;
  } else if (lowerKey == L"stage") {
    static constexpr const wchar_t* values[] = {
        L"first", L"full", L"cache", L"prepare", L"lookup", L"serialize", L"write", L"paint"};
    for (const wchar_t* safe : values) if (lowerValue == safe) return true;
  }
  return false;
}

inline std::wstring RedactDiagnosticMessage(const wchar_t* message) {
  if (!message || !*message) return L"(empty)";
  const std::wstring text(message);
  std::wstring out = L"redacted chars=" + std::to_wstring(text.size());
  size_t kept = 0;
  size_t pos = 0;
  while (pos < text.size() && kept < 16) {
    while (pos < text.size() && (text[pos] == L' ' || text[pos] == L',' || text[pos] == L';')) ++pos;
    const size_t start = pos;
    while (pos < text.size() && text[pos] != L' ' && text[pos] != L',' && text[pos] != L';') ++pos;
    if (start == pos) continue;
    const std::wstring token = TrimDiagnosticToken(text.substr(start, pos - start));
    const size_t eq = token.find(L'=');
    if (eq == std::wstring::npos || eq == 0) continue;
    const std::wstring key = TrimDiagnosticToken(token.substr(0, eq));
    if (!IsSafeDiagnosticKey(key) || !IsSafeDiagnosticValue(key, token.substr(eq + 1))) continue;
    out += L" " + token;
    ++kept;
  }
  return out;
}
