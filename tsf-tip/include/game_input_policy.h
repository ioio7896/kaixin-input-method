#pragma once
#include <cstdint>

enum class SrfGameInputMode { Manual, Passthrough, Chinese, AutoText };
enum class SrfGameEnterBehavior { Auto, Close, Stay };
enum class SrfGameChatPhase { Passive, Awaiting, Editing, Committing };
enum class SrfGameTextSource { None, Manual, Native, Tsf, Automation, AlwaysChinese };

class SrfGameCommitScope {
 public:
  SrfGameCommitScope(SrfGameChatPhase& phase, const bool& active) : phase_(phase), active_(active) {
    if (active_) phase_ = SrfGameChatPhase::Committing;
  }
  ~SrfGameCommitScope() {
    phase_ = active_ ? SrfGameChatPhase::Editing : SrfGameChatPhase::Passive;
  }
 private:
  SrfGameChatPhase& phase_;
  const bool& active_;
};

inline bool SrfGameExitOnEnter(SrfGameEnterBehavior behavior, bool verifiedEditable) {
  return behavior == SrfGameEnterBehavior::Close ||
         (behavior == SrfGameEnterBehavior::Auto && !verifiedEditable);
}

inline bool SrfCandidateHostMayShow(bool hostShow, bool explicitOverride) {
  return hostShow || explicitOverride;
}

inline bool SrfGameFocusEvidenceFresh(std::uint64_t checked, std::uint64_t now) {
  return checked != 0 && now >= checked && now - checked <= 350;
}

// A window class alone is shared by games, tools, editors and launchers.
inline bool SrfClassifyGame(bool explicitProfile, bool processMatch, bool /*classMatch*/) {
  return explicitProfile || processMatch;
}

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
