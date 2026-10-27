#pragma once
#include <windows.h>
#include <initializer_list>

inline constexpr ULONG_PTR kSrfInjectedInputMarker = 0x4b58494du;
// The touch keyboard sends these two private-use Unicode packets. Ordinary
// injected text must continue to bypass the IME to avoid recursive commits.
inline UINT SrfTouchKeyboardPageKey(wchar_t character, ULONG_PTR extra) {
  if (extra == kSrfInjectedInputMarker) return 0;
  if (character == 0xf003) return VK_NEXT;
  if (character == 0xf004) return VK_PRIOR;
  return 0;
}

inline UINT SrfReadTouchKeyboardPageKey(UINT vk) {
  if (vk != VK_PACKET) return 0;
  BYTE state[256] = {};
  WCHAR character = 0;
  if (!GetKeyboardState(state) || ToUnicode(VK_PACKET, 0, state, &character, 1, 0) != 1) return 0;
  return SrfTouchKeyboardPageKey(character, static_cast<ULONG_PTR>(GetMessageExtraInfo()));
}
inline bool SrfIsInjectedTextEvent(UINT vk, LPARAM extra) {
  return vk == VK_PACKET || static_cast<ULONG_PTR>(extra) == kSrfInjectedInputMarker;
}

inline bool SrfInjectionModifiersClear() {
  for (int key : {VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU, VK_RMENU, VK_SHIFT,
                  VK_LSHIFT, VK_RSHIFT, VK_LWIN, VK_RWIN}) {
    if (GetAsyncKeyState(key) & 0x8000) return false;
  }
  return true;
}
