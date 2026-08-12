#pragma once

#include <cstdint>

struct SrfInitialInputModeState {
  bool imeOpen = true;
  bool fullShape = false;
};

inline SrfInitialInputModeState ResolveInitialInputModeState(bool defaultAscii,
                                                             bool defaultFullShape) {
  // TSF input-mode compartments are shared by every input method on the
  // thread.  A newly activated TIP must start from its own configured defaults
  // instead of inheriting the previous IME's open/full-shape state.
  return {!defaultAscii, defaultFullShape};
}

inline std::uint32_t ClearSystemFullShapeConversion(std::uint32_t conversion,
                                                    std::uint32_t fullShapeMask) {
  // This TIP converts direct text itself while Chinese mode is open.  Leaving
  // the shared bit set lets the host widen Latin keys before they can become a
  // pinyin reading.
  return conversion & ~fullShapeMask;
}

inline bool ShouldRegisterFullShapeHotkey(bool defaultFullShape,
                                          bool fullShapeHotkeyEnabled) {
  // The default-full-shape setting promises Shift+Space as a temporary escape;
  // the explicit shortcut setting also makes it available from half shape.
  return defaultFullShape || fullShapeHotkeyEnabled;
}
