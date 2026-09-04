#pragma once

#include <windows.h>

namespace wintranslator_protocol {

inline constexpr unsigned kVersion = 2;
inline constexpr wchar_t kRequestPipe[] = LR"(\\.\pipe\WinTranslator.Request)";
inline constexpr DWORD kProbeTimeoutMs = 120;
inline constexpr DWORD kStartupTimeoutMs = 12'000;
inline constexpr size_t kMaximumRequestBytes = 1024 * 1024;

}  // namespace wintranslator_protocol
