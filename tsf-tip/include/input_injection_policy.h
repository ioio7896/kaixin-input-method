#pragma once
#include <windows.h>
#include <initializer_list>

inline constexpr ULONG_PTR kSrfInjectedInputMarker = 0x4b58494du;
inline bool SrfIsInjectedTextEvent(UINT vk, LPARAM extra) {
  return vk == VK_PACKET || static_cast<ULONG_PTR>(extra) == kSrfInjectedInputMarker;
}

inline bool SrfInjectionModifiersClear() {
  for (int key : {VK_CONTROL, VK_LCONTROL, VK_RCONTROL, VK_MENU, VK_LMENU,
                  VK_RMENU, VK_SHIFT, VK_LSHIFT, VK_RSHIFT, VK_LWIN, VK_RWIN}) {
    if (GetAsyncKeyState(key) & 0x8000) return false;
  }
  return true;
}
