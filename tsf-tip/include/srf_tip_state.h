#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

#include "candidate_metadata.h"
#include "game_focus_probe.h"
#include <msctf.h>

struct CandidateRow {
  std::wstring text;
  CandidateMetaParts meta;
};

struct CompositionState {
  void ClampCursor() {
    if (readingCursor > reading.size()) readingCursor = reading.size();
  }

  ITfComposition* pComposition = nullptr;
  ITfContext* pCompositionContext = nullptr;

  std::wstring reading;
  size_t readingCursor = 0;
  // 逐字拼词（单字上屏后保留剩余拼音）时，累计用户造的新词。
  // 目的：即使词库没有该多字词，用户也能逐字拼出，并在结束时把整词写入用户词典。
  bool userPhraseComposeActive = false;
  bool userPhraseComposeValid = false;
  std::wstring userPhraseComposeOriginalReading;
  std::wstring userPhraseComposeCommitted;
};

struct CandidateViewController {
  void ResetFullLookup() {
    candidateFullLookupPending = false;
    candidateFullLookupSerial = 0;
    candidatePageAfterLoad = 0;
    candidateIndexAfterLoad = 0;
  }

  bool CompleteFullLookup(unsigned long long serial) {
    if (!candidateFullLookupPending || serial != candidateFullLookupSerial) return false;
    candidateFullLookupPending = false;
    candidateFullLookupSerial = 0;
    return true;
  }

  std::vector<std::wstring> candidates;
  std::vector<CandidateMetaParts> candidateMeta;
  std::vector<CandidateRow> candidateRows;
  std::wstring candidatesReading;
  bool candidateHasMore = false;
  bool candidateFullLookupPending = false;
  unsigned long long candidateFullLookupSerial = 0;
  UINT candidatePageAfterLoad = 0;
  UINT candidateIndexAfterLoad = 0;
  std::wstring clipboardQuickLastPageReading;
  enum class SrfCandidateViewState : UINT {
    Empty = 0,
    Stable = 1,
    Pending = 2,
    Stale = 3,
    PartialCurrent = 4,
  };
  SrfCandidateViewState candidateViewState = SrfCandidateViewState::Empty;
  ULONGLONG candidateViewStateSinceTick = 0;
  UINT candSel = 0;
  UINT candPage = 0;
  UINT lastSyncedCandSel = 0;
  UINT lastSyncedCandPage = 0;
  unsigned long long candidateInteractionVersion = 0;
  unsigned long long partialCandidateInteractionVersion = 0;
  std::wstring partialCandidateInteractionReading;
};

struct AnchorStabilizer {
  void Reset() {
    hasLastCandidateRect = false;
    preserveCandidateAnchorReading.clear();
    hasStickyCandidateRect = false;
    stickyCandidateRect = {};
    stickyCandidateAnchorQuality = 0;
    lastCandidateAnchorSource.clear();
    lastCandidateAnchorQuality = 0;
    lastCandidateAnchorSourceSwitchTick = 0;
    candidateAnchorSourceSwitchCount = 0;
  }

  RECT lastCandidateRect = {};
  bool hasLastCandidateRect = false;
  // 逐字组词提交首字时，保留当前锚点直到“剩余拼音”的首批候选落地，
  // 避免候选窗先消失、再因新 composition 的临时坐标跳位。
  std::wstring preserveCandidateAnchorReading;
  RECT stickyCandidateRect = {};
  bool hasStickyCandidateRect = false;
  UINT stickyCandidateAnchorQuality = 0;
  std::wstring lastCandidateAnchorSource;
  UINT lastCandidateAnchorQuality = 0;
  ULONGLONG lastCandidateAnchorSourceSwitchTick = 0;
  unsigned int candidateAnchorSourceSwitchCount = 0;
};

struct GameChatPolicyState {
  bool chatActive = false;
  SrfGameChatPhase chatPhase = SrfGameChatPhase::Passive;
  SrfGameTextSource chatSource = SrfGameTextSource::None;
  std::uint64_t editableIdentity = 0;
  std::uint64_t chatEditableIdentity = 0;
  std::uint64_t dismissedIdentity = 0;
  RECT editableBounds = {};
  UINT candidateFieldWidthDip = 0;
  bool editableVerified = false;
  SrfGameFocusEvidence tsfEvidence = {};
  ULONGLONG tsfProbeTick = 0;
  bool tsfProbePending = false;
  std::uint64_t tsfProbeSerial = 0;
  ULONGLONG awaitingUntil = 0;
  ULONGLONG awaitingHintAt = 0;
  struct LearnedGameField {
    std::wstring app;
    std::vector<std::wstring> windowClasses;
    DWORD processId = 0;
    std::uint64_t element = 0;
  };
  std::vector<LearnedGameField> learnedFields;
  HWND lastNativeFocus = nullptr;
  HWND learningTarget = nullptr;
  DWORD learningProcessId = 0;
  std::uint64_t learnedNativeSignature = 0;
  HWND openingFocus = nullptr;
  HWND geometryFocus = nullptr;
  RECT geometryBounds = {};
  ULONGLONG geometrySince = 0;
  ULONGLONG chatOpenedAt = 0;
  ULONGLONG openAttemptAt = 0;
  HWND nativeEdit = nullptr;
  std::wstring pendingPaste;
  bool chatSavedImeOpen = true;
  HWND chatOwner = nullptr;
  DWORD chatOwnerProcessId = 0;
  HWND autoGameChatFocus = nullptr;
  HWND autoGameChatDismissedFocus = nullptr;
};
