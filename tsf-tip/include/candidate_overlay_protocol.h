#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <cstdint>
#include <cstring>
#include <limits>
#include <string>
#include <utility>
#include <vector>

// Candidate snapshots are deliberately sent to a separate, non-injected
// process.  Keep this wire format pointer-size independent so x86 and x64
// helpers can be tested with the same fixtures.
inline constexpr ULONG_PTR kSrfCandidateOverlayCopyDataId =
    static_cast<ULONG_PTR>(0x4B584F56u);  // "KXOV"
inline constexpr ULONG_PTR kSrfCandidateOverlayStatusCopyDataId =
    static_cast<ULONG_PTR>(0x4B584F53u);  // "KXOS"
inline constexpr std::uint32_t kSrfCandidateOverlayMagic = 0x564F584Bu;
inline constexpr std::uint32_t kSrfCandidateOverlayStatusMagic = 0x534F584Bu;
inline constexpr std::uint16_t kSrfCandidateOverlayVersion = 2;
inline constexpr std::size_t kSrfCandidateOverlayMaxPacketBytes = 256u * 1024u;
inline constexpr std::size_t kSrfCandidateOverlayMaxTextUnits = 32u * 1024u;
inline constexpr std::size_t kSrfCandidateOverlayMaxItems = 32u;

inline const wchar_t* SrfCandidateOverlayClientWindowClass() {
  return L"SRF_TSF_DeferredTimer";
}

inline UINT SrfCandidateOverlayStatusChangedMessage() {
  // Avoid colliding with another WM_APP user of the shared deferred window.
  // RegisterWindowMessage gives the TIP and its helper the same session-wide
  // message id without reserving a fixed application-message slot.
  static const UINT message = RegisterWindowMessageW(
      L"SRF_IME_Candidate_Overlay_Status_Changed_v2");
  return message;
}

enum SrfCandidateOverlayFlags : std::uint32_t {
  kSrfCandidateOverlayVisible = 1u << 0,
  kSrfCandidateOverlayPending = 1u << 1,
  kSrfCandidateOverlayGameCompact = 1u << 2,
  kSrfCandidateOverlayFullscreen = 1u << 3,
  kSrfCandidateOverlayLayoutResolved = 1u << 4,
  kSrfCandidateOverlayHorizontal = 1u << 5,
  kSrfCandidateOverlayHorizontalCompact = 1u << 6,
  kSrfCandidateOverlayAnchorPhysical = 1u << 7,
  kSrfCandidateOverlayCaretAnchor = 1u << 8,
};

struct SrfCandidateOverlayAuthSecret {
  std::uint64_t high = 0;
  std::uint64_t low = 0;

  bool Valid() const { return high != 0 || low != 0; }
};

#pragma pack(push, 1)
struct SrfCandidateOverlayWireHeader {
  std::uint32_t magic = kSrfCandidateOverlayMagic;
  std::uint16_t version = kSrfCandidateOverlayVersion;
  std::uint16_t headerBytes = sizeof(SrfCandidateOverlayWireHeader);
  std::uint32_t totalBytes = 0;
  std::uint32_t flags = 0;
  std::uint32_t sourceProcessId = 0;
  std::uint32_t targetProcessId = 0;
  std::uint64_t ownerId = 0;
  std::uint64_t authSecretHigh = 0;
  std::uint64_t authSecretLow = 0;
  std::uint64_t targetHwnd = 0;
  std::uint64_t focusGeneration = 0;
  std::uint64_t sequence = 0;
  std::int32_t anchorLeft = 0;
  std::int32_t anchorTop = 0;
  std::int32_t anchorRight = 0;
  std::int32_t anchorBottom = 0;
  std::uint32_t pageIndex = 0;
  std::uint32_t totalPages = 0;
  std::uint32_t selectedInPage = 0;
  std::uint32_t itemCount = 0;
  std::uint32_t commentCount = 0;
  std::uint32_t labelCount = 0;
  std::uint32_t pinnedCount = 0;
  std::uint32_t clipboardCount = 0;
  std::uint32_t modeTagCount = 0;
};
#pragma pack(pop)

static_assert(sizeof(SrfCandidateOverlayWireHeader) == 124,
              "candidate overlay wire header changed unexpectedly");

#pragma pack(push, 1)
struct SrfCandidateOverlayStatusQuery {
  std::uint32_t magic = kSrfCandidateOverlayStatusMagic;
  std::uint16_t version = kSrfCandidateOverlayVersion;
  std::uint16_t bytes = sizeof(SrfCandidateOverlayStatusQuery);
  std::uint32_t sourceProcessId = 0;
  std::uint32_t reserved = 0;
  std::uint64_t ownerId = 0;
  std::uint64_t sequence = 0;
  std::uint64_t authSecretHigh = 0;
  std::uint64_t authSecretLow = 0;
};
#pragma pack(pop)

static_assert(sizeof(SrfCandidateOverlayStatusQuery) == 48,
              "candidate overlay status query changed unexpectedly");

enum class SrfCandidateOverlayStatus : std::uint32_t {
  Unavailable = 0,
  // The queried sequence is still queued, or an older frame for the same
  // owner remains visible while the helper applies it.
  OwnerVisible = 1,
  SequenceApplied = 2,
  // A newer accepted sequence belongs to another owner. The old owner must
  // stop health-driven retries until a real focus/candidate update occurs.
  Superseded = 3,
};

struct SrfCandidateOverlayStatusState {
  std::uint64_t queryOwnerId = 0;
  std::uint64_t querySequence = 0;
  std::uint64_t lastAcceptedOwnerId = 0;
  std::uint64_t lastAcceptedSequence = 0;
  std::uint64_t pendingOwnerId = 0;
  std::uint64_t pendingSequence = 0;
  std::uint64_t activeOwnerId = 0;
  std::uint64_t lastAppliedSequence = 0;
  bool activeWindowVisible = false;
};

inline SrfCandidateOverlayStatus ResolveCandidateOverlayStatus(
    const SrfCandidateOverlayStatusState& state) {
  if (state.queryOwnerId == 0 || state.querySequence == 0) {
    return SrfCandidateOverlayStatus::Unavailable;
  }
  if (state.lastAcceptedSequence > state.querySequence &&
      state.lastAcceptedOwnerId != 0 &&
      state.lastAcceptedOwnerId != state.queryOwnerId) {
    return SrfCandidateOverlayStatus::Superseded;
  }

  const bool sequencePending =
      state.pendingOwnerId == state.queryOwnerId &&
      state.pendingSequence >= state.querySequence;
  if (state.activeOwnerId != state.queryOwnerId ||
      !state.activeWindowVisible) {
    return sequencePending ? SrfCandidateOverlayStatus::OwnerVisible
                           : SrfCandidateOverlayStatus::Unavailable;
  }
  return state.lastAppliedSequence >= state.querySequence
             ? SrfCandidateOverlayStatus::SequenceApplied
             : SrfCandidateOverlayStatus::OwnerVisible;
}

struct SrfCandidateOverlaySnapshot {
  bool visible = false;
  bool pendingVisual = false;
  bool gameCompact = false;
  bool fullscreenPlacement = false;
  bool layoutResolved = false;
  bool horizontalLayout = false;
  bool horizontalCompact = false;
  bool anchorPhysical = false;
  // Dynamic TSF caret anchors must be re-sampled in the source process after
  // DPI/display/window-mode changes. Fixed game anchors can be recomputed by
  // the independent helper itself.
  bool caretAnchor = false;
  DWORD sourceProcessId = 0;
  DWORD targetProcessId = 0;
  std::uint64_t ownerId = 0;
  SrfCandidateOverlayAuthSecret authSecret = {};
  HWND targetHwnd = nullptr;
  std::uint64_t focusGeneration = 0;
  std::uint64_t sequence = 0;
  RECT anchor = {};
  UINT pageIndex = 0;
  UINT totalPages = 0;
  UINT selectedInPage = 0;
  std::wstring appPath;
  std::wstring title;
  std::vector<std::wstring> items;
  std::vector<std::wstring> comments;
  std::vector<std::wstring> labels;
  std::vector<bool> pinnedItems;
  std::vector<bool> clipboardItems;
  std::vector<std::wstring> modeTags;
};

namespace srf_candidate_overlay_wire {

inline bool AppendBytes(std::vector<std::uint8_t>* output, const void* data,
                        std::size_t bytes) {
  if (!output || (!data && bytes != 0)) return false;
  if (bytes == 0) return true;
  if (bytes > kSrfCandidateOverlayMaxPacketBytes ||
      output->size() > kSrfCandidateOverlayMaxPacketBytes - bytes) {
    return false;
  }
  const auto* first = static_cast<const std::uint8_t*>(data);
  output->insert(output->end(), first, first + bytes);
  return true;
}

inline bool AppendString(std::vector<std::uint8_t>* output, const std::wstring& value) {
  if (value.size() > kSrfCandidateOverlayMaxTextUnits ||
      value.size() > (std::numeric_limits<std::uint32_t>::max)()) {
    return false;
  }
  const std::uint32_t length = static_cast<std::uint32_t>(value.size());
  return AppendBytes(output, &length, sizeof(length)) &&
         AppendBytes(output, value.data(), value.size() * sizeof(wchar_t));
}

inline bool AppendStrings(std::vector<std::uint8_t>* output,
                          const std::vector<std::wstring>& values) {
  if (values.size() > kSrfCandidateOverlayMaxItems) return false;
  for (const auto& value : values) {
    if (!AppendString(output, value)) return false;
  }
  return true;
}

inline bool AppendBools(std::vector<std::uint8_t>* output,
                        const std::vector<bool>& values) {
  if (values.size() > kSrfCandidateOverlayMaxItems) return false;
  for (bool value : values) {
    const std::uint8_t byte = value ? 1u : 0u;
    if (!AppendBytes(output, &byte, sizeof(byte))) return false;
  }
  return true;
}

class Reader {
 public:
  Reader(const void* data, std::size_t bytes)
      : current_(static_cast<const std::uint8_t*>(data)), remaining_(bytes) {}

  bool ReadBytes(void* output, std::size_t bytes) {
    if ((!output && bytes != 0) || bytes > remaining_) return false;
    if (bytes != 0) std::memcpy(output, current_, bytes);
    current_ += bytes;
    remaining_ -= bytes;
    return true;
  }

  bool ReadString(std::wstring* output) {
    if (!output) return false;
    std::uint32_t length = 0;
    if (!ReadBytes(&length, sizeof(length)) ||
        length > kSrfCandidateOverlayMaxTextUnits) {
      return false;
    }
    const std::size_t bytes = static_cast<std::size_t>(length) * sizeof(wchar_t);
    if (bytes > remaining_) return false;
    output->assign(reinterpret_cast<const wchar_t*>(current_), length);
    current_ += bytes;
    remaining_ -= bytes;
    return true;
  }

  bool ReadStrings(std::uint32_t count, std::vector<std::wstring>* output) {
    if (!output || count > kSrfCandidateOverlayMaxItems) return false;
    output->clear();
    output->reserve(count);
    for (std::uint32_t i = 0; i < count; ++i) {
      std::wstring value;
      if (!ReadString(&value)) return false;
      output->push_back(std::move(value));
    }
    return true;
  }

  bool ReadBools(std::uint32_t count, std::vector<bool>* output) {
    if (!output || count > kSrfCandidateOverlayMaxItems) return false;
    output->clear();
    output->reserve(count);
    for (std::uint32_t i = 0; i < count; ++i) {
      std::uint8_t value = 0;
      if (!ReadBytes(&value, sizeof(value)) || value > 1u) return false;
      output->push_back(value != 0);
    }
    return true;
  }

  bool Empty() const { return remaining_ == 0; }

 private:
  const std::uint8_t* current_ = nullptr;
  std::size_t remaining_ = 0;
};

}  // namespace srf_candidate_overlay_wire

inline bool SerializeCandidateOverlaySnapshot(
    const SrfCandidateOverlaySnapshot& snapshot, std::vector<std::uint8_t>* output) {
  if (!output || snapshot.items.size() > kSrfCandidateOverlayMaxItems ||
      snapshot.comments.size() > kSrfCandidateOverlayMaxItems ||
      snapshot.labels.size() > kSrfCandidateOverlayMaxItems ||
      snapshot.pinnedItems.size() > kSrfCandidateOverlayMaxItems ||
      snapshot.clipboardItems.size() > kSrfCandidateOverlayMaxItems ||
      snapshot.modeTags.size() > kSrfCandidateOverlayMaxItems) {
    return false;
  }

  SrfCandidateOverlayWireHeader header = {};
  header.flags = (snapshot.visible ? kSrfCandidateOverlayVisible : 0u) |
                 (snapshot.pendingVisual ? kSrfCandidateOverlayPending : 0u) |
                 (snapshot.gameCompact ? kSrfCandidateOverlayGameCompact : 0u) |
                 (snapshot.fullscreenPlacement ? kSrfCandidateOverlayFullscreen : 0u) |
                 (snapshot.layoutResolved ? kSrfCandidateOverlayLayoutResolved : 0u) |
                 (snapshot.horizontalLayout ? kSrfCandidateOverlayHorizontal : 0u) |
                 (snapshot.horizontalCompact
                      ? kSrfCandidateOverlayHorizontalCompact
                      : 0u) |
                 (snapshot.anchorPhysical ? kSrfCandidateOverlayAnchorPhysical : 0u) |
                 (snapshot.caretAnchor ? kSrfCandidateOverlayCaretAnchor : 0u);
  header.sourceProcessId = snapshot.sourceProcessId;
  header.targetProcessId = snapshot.targetProcessId;
  header.ownerId = snapshot.ownerId;
  header.authSecretHigh = snapshot.authSecret.high;
  header.authSecretLow = snapshot.authSecret.low;
  header.targetHwnd = static_cast<std::uint64_t>(
      reinterpret_cast<std::uintptr_t>(snapshot.targetHwnd));
  header.focusGeneration = snapshot.focusGeneration;
  header.sequence = snapshot.sequence;
  header.anchorLeft = snapshot.anchor.left;
  header.anchorTop = snapshot.anchor.top;
  header.anchorRight = snapshot.anchor.right;
  header.anchorBottom = snapshot.anchor.bottom;
  header.pageIndex = snapshot.pageIndex;
  header.totalPages = snapshot.totalPages;
  header.selectedInPage = snapshot.selectedInPage;
  header.itemCount = static_cast<std::uint32_t>(snapshot.items.size());
  header.commentCount = static_cast<std::uint32_t>(snapshot.comments.size());
  header.labelCount = static_cast<std::uint32_t>(snapshot.labels.size());
  header.pinnedCount = static_cast<std::uint32_t>(snapshot.pinnedItems.size());
  header.clipboardCount = static_cast<std::uint32_t>(snapshot.clipboardItems.size());
  header.modeTagCount = static_cast<std::uint32_t>(snapshot.modeTags.size());

  output->clear();
  output->reserve(sizeof(header) + 1024);
  if (!srf_candidate_overlay_wire::AppendBytes(output, &header, sizeof(header)) ||
      !srf_candidate_overlay_wire::AppendString(output, snapshot.appPath) ||
      !srf_candidate_overlay_wire::AppendString(output, snapshot.title) ||
      !srf_candidate_overlay_wire::AppendStrings(output, snapshot.items) ||
      !srf_candidate_overlay_wire::AppendStrings(output, snapshot.comments) ||
      !srf_candidate_overlay_wire::AppendStrings(output, snapshot.labels) ||
      !srf_candidate_overlay_wire::AppendBools(output, snapshot.pinnedItems) ||
      !srf_candidate_overlay_wire::AppendBools(output, snapshot.clipboardItems) ||
      !srf_candidate_overlay_wire::AppendStrings(output, snapshot.modeTags)) {
    output->clear();
    return false;
  }
  if (output->size() > (std::numeric_limits<std::uint32_t>::max)()) {
    output->clear();
    return false;
  }
  header.totalBytes = static_cast<std::uint32_t>(output->size());
  std::memcpy(output->data(), &header, sizeof(header));
  return true;
}

inline bool DeserializeCandidateOverlaySnapshot(
    const void* data, std::size_t bytes, SrfCandidateOverlaySnapshot* snapshot) {
  if (!data || !snapshot || bytes < sizeof(SrfCandidateOverlayWireHeader) ||
      bytes > kSrfCandidateOverlayMaxPacketBytes) {
    return false;
  }
  SrfCandidateOverlayWireHeader header = {};
  std::memcpy(&header, data, sizeof(header));
  if (header.magic != kSrfCandidateOverlayMagic ||
      header.version != kSrfCandidateOverlayVersion ||
      header.headerBytes != sizeof(SrfCandidateOverlayWireHeader) ||
      header.totalBytes != bytes || header.itemCount > kSrfCandidateOverlayMaxItems ||
      header.commentCount > kSrfCandidateOverlayMaxItems ||
      header.labelCount > kSrfCandidateOverlayMaxItems ||
      header.pinnedCount > kSrfCandidateOverlayMaxItems ||
      header.clipboardCount > kSrfCandidateOverlayMaxItems ||
      header.modeTagCount > kSrfCandidateOverlayMaxItems) {
    return false;
  }

  srf_candidate_overlay_wire::Reader reader(
      static_cast<const std::uint8_t*>(data) + sizeof(header), bytes - sizeof(header));
  SrfCandidateOverlaySnapshot parsed = {};
  parsed.visible = (header.flags & kSrfCandidateOverlayVisible) != 0;
  parsed.pendingVisual = (header.flags & kSrfCandidateOverlayPending) != 0;
  parsed.gameCompact = (header.flags & kSrfCandidateOverlayGameCompact) != 0;
  parsed.fullscreenPlacement =
      (header.flags & kSrfCandidateOverlayFullscreen) != 0;
  parsed.layoutResolved =
      (header.flags & kSrfCandidateOverlayLayoutResolved) != 0;
  parsed.horizontalLayout =
      (header.flags & kSrfCandidateOverlayHorizontal) != 0;
  parsed.horizontalCompact =
      (header.flags & kSrfCandidateOverlayHorizontalCompact) != 0;
  parsed.anchorPhysical =
      (header.flags & kSrfCandidateOverlayAnchorPhysical) != 0;
  parsed.caretAnchor =
      (header.flags & kSrfCandidateOverlayCaretAnchor) != 0;
  parsed.sourceProcessId = header.sourceProcessId;
  parsed.targetProcessId = header.targetProcessId;
  parsed.ownerId = header.ownerId;
  parsed.authSecret = {header.authSecretHigh, header.authSecretLow};
  parsed.targetHwnd = reinterpret_cast<HWND>(
      static_cast<std::uintptr_t>(header.targetHwnd));
  parsed.focusGeneration = header.focusGeneration;
  parsed.sequence = header.sequence;
  parsed.anchor = {header.anchorLeft, header.anchorTop, header.anchorRight,
                   header.anchorBottom};
  parsed.pageIndex = header.pageIndex;
  parsed.totalPages = header.totalPages;
  parsed.selectedInPage = header.selectedInPage;
  if (!reader.ReadString(&parsed.appPath) || !reader.ReadString(&parsed.title) ||
      !reader.ReadStrings(header.itemCount, &parsed.items) ||
      !reader.ReadStrings(header.commentCount, &parsed.comments) ||
      !reader.ReadStrings(header.labelCount, &parsed.labels) ||
      !reader.ReadBools(header.pinnedCount, &parsed.pinnedItems) ||
      !reader.ReadBools(header.clipboardCount, &parsed.clipboardItems) ||
      !reader.ReadStrings(header.modeTagCount, &parsed.modeTags) || !reader.Empty()) {
    return false;
  }
  if (parsed.visible &&
      (parsed.items.empty() || parsed.comments.size() != parsed.items.size() ||
       parsed.labels.size() != parsed.items.size() ||
       parsed.pinnedItems.size() != parsed.items.size() ||
       parsed.clipboardItems.size() != parsed.items.size() ||
       parsed.selectedInPage >= parsed.items.size())) {
    return false;
  }
  *snapshot = std::move(parsed);
  return true;
}

inline const wchar_t* SrfCandidateOverlayControlWindowClass() {
#if defined(_WIN64)
  return L"SRF_IME_Candidate_Overlay_Control_v2_x64";
#else
  return L"SRF_IME_Candidate_Overlay_Control_v2_x86";
#endif
}
