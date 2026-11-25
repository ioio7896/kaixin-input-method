#pragma once
#include <cstdint>

enum class SrfGameInputMode { Manual, Passthrough, Chinese, AutoText };
enum class SrfGameEnterBehavior { Auto, Close, Stay };
enum class SrfGameChatPhase { Passive, Awaiting, Editing, Committing };
enum class SrfGameTextSource {
  None,
  Manual,
  Native,
  Tsf,
  Automation,
  Learned,
  Geometry,
  AlwaysChinese
};

inline SrfGameInputMode SrfEffectiveGameInputMode(bool pureGameMode, SrfGameInputMode configured) {
  return pureGameMode ? SrfGameInputMode::Passthrough : configured;
}

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

inline bool SrfGameTextEvidenceReady(std::uint64_t checked, std::uint64_t stableSince,
                                     bool needsStability, bool learned) {
  return !needsStability || learned ||
         (stableSince != 0 && checked >= stableSince && checked - stableSince >= 100);
}

// A window class alone is shared by games, tools, editors and launchers.
inline bool SrfClassifyGame(bool explicitProfile, bool processMatch, bool /*classMatch*/) {
  return explicitProfile || processMatch;
}

inline bool SrfGameShouldPassThrough(bool game, SrfGameInputMode mode, bool chat, bool editable) {
  if (!game) return false;
  if (mode == SrfGameInputMode::Passthrough) return true;
  if (chat || mode == SrfGameInputMode::Chinese) return false;
  return mode != SrfGameInputMode::AutoText || !editable;
}

inline bool SrfGameKeepHeldKey(bool passedDown, bool repeated) {
  return passedDown && repeated;
}

inline bool SrfGameShouldExitChat(bool game, bool chat, bool reading, bool exitKey, bool modified,
                                  bool repeated) {
  return game && chat && !reading && exitKey && !modified && !repeated;
}

inline bool SrfShouldRetryKeyEditSession(bool callbackInvoked) {
  return !callbackInvoked;
}

enum class SrfShiftTapAction {
  None,
  ToggleImeOpen,
  ToggleGameChat,
  HintGamePassthrough,
};

inline bool SrfGameAllowChatImeHotkeys(bool game, bool pureGameMode, SrfGameInputMode mode,
                                       bool chatActive) {
  if (!game) return true;
  if (pureGameMode || mode == SrfGameInputMode::Passthrough) return false;
  return chatActive || mode == SrfGameInputMode::Chinese;
}

inline SrfShiftTapAction SrfResolveShiftTapAction(bool shiftTapEnabled, bool pureGameMode,
                                                  bool gameActive, SrfGameInputMode gameMode,
                                                  bool gameChatActive, bool gameShiftToggle,
                                                  bool forceAsciiNonGame, bool imeHotkeysAllowed) {
  if (!shiftTapEnabled || pureGameMode) return SrfShiftTapAction::None;
  if (gameActive) {
    if (gameMode == SrfGameInputMode::Passthrough) return SrfShiftTapAction::None;
    if (gameChatActive || gameMode == SrfGameInputMode::Chinese) {
      return SrfShiftTapAction::ToggleImeOpen;
    }
    if (gameShiftToggle) return SrfShiftTapAction::ToggleGameChat;
    return SrfShiftTapAction::HintGamePassthrough;
  }
  if (forceAsciiNonGame || !imeHotkeysAllowed) return SrfShiftTapAction::None;
  return SrfShiftTapAction::ToggleImeOpen;
}

class SrfShiftTapTracker {
 public:
  void Reset() {
    active_ = false;
    tainted_ = false;
    downTick_ = 0;
  }

  void ResetHints() {
    lastHintTapTick_ = 0;
    lastHintShownTick_ = 0;
  }

  void ObserveKeyDown(bool isShift, bool repeated, bool otherModifiersDown, bool mouseDown,
                      std::uint64_t now) {
    if (isShift) {
      if (repeated || active_ || otherModifiersDown || mouseDown) {
        tainted_ = true;
      } else {
        active_ = true;
        tainted_ = false;
        downTick_ = now;
      }
      return;
    }
    if (active_) {
      tainted_ = true;
    }
  }

  bool ObserveShiftUp(bool otherModifiersDown, bool mouseInteracted, std::uint64_t now,
                      std::uint64_t maxTapMs = 420) {
    const bool pureTap = active_ && !tainted_ && !otherModifiersDown && !mouseInteracted &&
                         now >= downTick_ && (now - downTick_) <= maxTapMs;
    active_ = false;
    tainted_ = false;
    downTick_ = 0;
    return pureTap;
  }

  bool ObservePassthroughHintTap(std::uint64_t now, std::uint64_t doubleTapWindowMs = 450,
                                 std::uint64_t cooldownMs = 15000) {
    const bool isDoubleTap = lastHintTapTick_ != 0 && now >= lastHintTapTick_ &&
                             (now - lastHintTapTick_) <= doubleTapWindowMs;
    lastHintTapTick_ = now;
    if (!isDoubleTap) return false;
    lastHintTapTick_ = 0;
    if (lastHintShownTick_ != 0 && now >= lastHintShownTick_ &&
        (now - lastHintShownTick_) < cooldownMs) {
      return false;
    }
    lastHintShownTick_ = now;
    return true;
  }

  bool Active() const { return active_; }

 private:
  bool active_ = false;
  bool tainted_ = false;
  std::uint64_t downTick_ = 0;
  std::uint64_t lastHintTapTick_ = 0;
  std::uint64_t lastHintShownTick_ = 0;
};
