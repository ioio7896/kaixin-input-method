#pragma once

enum class SrfGameInputMode { Manual, Passthrough, Chinese, AutoText };

inline bool SrfGameShouldPassThrough(bool game, SrfGameInputMode mode, bool chat,
                                    bool editable) {
  if (!game) return false;
  if (mode == SrfGameInputMode::Passthrough) return true;
  if (chat || mode == SrfGameInputMode::Chinese) return false;
  return mode != SrfGameInputMode::AutoText || !editable;
}

inline bool SrfGameKeepHeldKey(bool passedDown, bool repeated) {
  return passedDown && repeated;
}

inline bool SrfGameShouldExitChat(bool game, bool chat, bool reading, bool exitKey,
                                  bool modified, bool repeated) {
  return game && chat && !reading && exitKey && !modified && !repeated;
}

inline bool SrfShouldRetryKeyEditSession(bool callbackInvoked) {
  return !callbackInvoked;
}
