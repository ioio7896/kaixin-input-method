#include "candidate_overlay_protocol.h"

#include <cstdint>
#include <vector>

namespace {

bool SameRect(const RECT& left, const RECT& right) {
  return left.left == right.left && left.top == right.top &&
         left.right == right.right && left.bottom == right.bottom;
}

}  // namespace

int main() {
  SrfCandidateOverlaySnapshot input = {};
  input.visible = true;
  input.pendingVisual = true;
  input.gameCompact = true;
  input.fullscreenPlacement = true;
  input.layoutResolved = true;
  input.horizontalLayout = true;
  input.horizontalCompact = true;
  input.anchorPhysical = true;
  input.caretAnchor = true;
  input.sourceProcessId = 41;
  input.targetProcessId = 42;
  input.ownerId = 17;
  input.authSecret = {0x1122334455667788ull, 0x8877665544332211ull};
  input.targetHwnd = reinterpret_cast<HWND>(static_cast<std::uintptr_t>(0x12345678u));
  input.focusGeneration = 19;
  input.sequence = 27;
  input.anchor = {100, 200, 104, 220};
  input.pageIndex = 2;
  input.totalPages = 3;
  input.selectedInPage = 1;
  input.appPath = L"C:\\Games\\Example\\game.exe";
  input.title = L"nihao";
  input.items = {L"你好", L"拟好", L"你号"};
  input.comments = {L"", L"候选", L""};
  input.labels = {L"1", L"2", L"3"};
  input.pinnedItems = {true, false, false};
  input.clipboardItems = {false, false, true};
  input.modeTags = {L"游戏", L"全屏"};

  std::vector<std::uint8_t> bytes;
  if (!SerializeCandidateOverlaySnapshot(input, &bytes) || bytes.empty()) return 1;

  SrfCandidateOverlaySnapshot output = {};
  if (!DeserializeCandidateOverlaySnapshot(bytes.data(), bytes.size(), &output)) return 2;
  if (!output.visible || !output.pendingVisual || !output.gameCompact ||
      !output.fullscreenPlacement || !output.layoutResolved ||
      !output.horizontalLayout || !output.horizontalCompact ||
      !output.anchorPhysical || !output.caretAnchor) {
    return 3;
  }
  if (output.sourceProcessId != input.sourceProcessId ||
      output.targetProcessId != input.targetProcessId ||
      output.ownerId != input.ownerId ||
      output.authSecret.high != input.authSecret.high ||
      output.authSecret.low != input.authSecret.low ||
      output.targetHwnd != input.targetHwnd ||
      output.focusGeneration != input.focusGeneration || output.sequence != input.sequence ||
      !SameRect(output.anchor, input.anchor)) {
    return 4;
  }
  if (output.pageIndex != input.pageIndex || output.totalPages != input.totalPages ||
      output.selectedInPage != input.selectedInPage || output.appPath != input.appPath ||
      output.title != input.title || output.items != input.items ||
      output.comments != input.comments || output.labels != input.labels ||
      output.pinnedItems != input.pinnedItems ||
      output.clipboardItems != input.clipboardItems || output.modeTags != input.modeTags) {
    return 5;
  }

  // Truncation, unknown versions, and inconsistent visible arrays must fail
  // closed instead of leaving stale game UI on screen.
  if (DeserializeCandidateOverlaySnapshot(bytes.data(), bytes.size() - 1, &output)) return 6;
  auto damaged = bytes;
  SrfCandidateOverlayWireHeader damagedHeader = {};
  std::memcpy(&damagedHeader, damaged.data(), sizeof(damagedHeader));
  damagedHeader.version += 1;
  std::memcpy(damaged.data(), &damagedHeader, sizeof(damagedHeader));
  if (DeserializeCandidateOverlaySnapshot(damaged.data(), damaged.size(), &output)) return 7;

  SrfCandidateOverlaySnapshot invalid = input;
  invalid.labels.pop_back();
  if (!SerializeCandidateOverlaySnapshot(invalid, &bytes)) return 8;
  if (DeserializeCandidateOverlaySnapshot(bytes.data(), bytes.size(), &output)) return 9;

  SrfCandidateOverlaySnapshot hidden = {};
  hidden.sourceProcessId = 1;
  hidden.targetProcessId = 2;
  hidden.focusGeneration = 30;
  hidden.sequence = 31;
  if (!SerializeCandidateOverlaySnapshot(hidden, &bytes) ||
      !DeserializeCandidateOverlaySnapshot(bytes.data(), bytes.size(), &output) ||
      output.visible) {
    return 10;
  }

  SrfCandidateOverlayStatusState status = {};
  status.queryOwnerId = 2;
  status.querySequence = 20;
  status.lastAcceptedOwnerId = 2;
  status.lastAcceptedSequence = 20;
  status.pendingOwnerId = 2;
  status.pendingSequence = 20;
  if (ResolveCandidateOverlayStatus(status) !=
      SrfCandidateOverlayStatus::OwnerVisible) {
    return 11;
  }

  status.pendingOwnerId = 0;
  status.pendingSequence = 0;
  status.activeOwnerId = 2;
  status.lastAppliedSequence = 20;
  status.activeWindowVisible = true;
  if (ResolveCandidateOverlayStatus(status) !=
      SrfCandidateOverlayStatus::SequenceApplied) {
    return 12;
  }

  // A newer owner supersedes the older owner both while its Show is pending
  // and after it immediately withdraws that lease with Hide.
  status.queryOwnerId = 1;
  status.querySequence = 10;
  status.lastAcceptedOwnerId = 2;
  status.lastAcceptedSequence = 20;
  status.activeOwnerId = 1;
  status.lastAppliedSequence = 10;
  if (ResolveCandidateOverlayStatus(status) !=
      SrfCandidateOverlayStatus::Superseded) {
    return 13;
  }
  status.lastAcceptedSequence = 21;
  status.activeOwnerId = 0;
  status.activeWindowVisible = false;
  if (ResolveCandidateOverlayStatus(status) !=
      SrfCandidateOverlayStatus::Superseded) {
    return 14;
  }

  status.queryOwnerId = 2;
  status.querySequence = 22;
  if (ResolveCandidateOverlayStatus(status) !=
      SrfCandidateOverlayStatus::Unavailable) {
    return 15;
  }
  return 0;
}
