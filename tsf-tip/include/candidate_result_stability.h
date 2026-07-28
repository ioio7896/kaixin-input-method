#pragma once

#include <algorithm>
#include <cstddef>
#include <cwctype>
#include <string>
#include <vector>

namespace srf_candidate_stability {

inline constexpr size_t kCandidateBatchLimit = 128;

inline bool CompletedBatchKeepsTop(const std::wstring& currentTop,
                                   const std::wstring& completedTop) {
  return !currentTop.empty() && currentTop == completedTop;
}

inline bool ShouldRetainEmptyCandidateResult(bool hasReading, bool nextCandidatesEmpty,
                                             bool currentCandidatesEmpty,
                                             bool transientLookup) {
  return hasReading && nextCandidatesEmpty && !currentCandidatesEmpty && transientLookup;
}

inline bool CanRefreshCompletedPartialSnapshot(bool currentIsPartial,
                                               bool snapshotReadingMatches,
                                               unsigned long long interactionVersion,
                                               unsigned long long snapshotInteractionVersion) {
  return currentIsPartial && snapshotReadingMatches &&
         interactionVersion == snapshotInteractionVersion;
}

inline std::wstring RemovePartialMetaFlag(const std::wstring& raw) {
  std::wstring out;
  size_t start = 0;
  while (start <= raw.size()) {
    const size_t end = raw.find(L'\t', start);
    const size_t length = end == std::wstring::npos ? raw.size() - start : end - start;
    const std::wstring token = raw.substr(start, length);

    size_t trimmedStart = 0;
    while (trimmedStart < token.size() && iswspace(token[trimmedStart])) ++trimmedStart;
    size_t trimmedEnd = token.size();
    while (trimmedEnd > trimmedStart && iswspace(token[trimmedEnd - 1])) --trimmedEnd;
    const std::wstring trimmed = token.substr(trimmedStart, trimmedEnd - trimmedStart);
    if (trimmed != L"partial" && trimmed != L"partial=1") {
      if (!out.empty()) out.push_back(L'\t');
      out.append(token);
    }

    if (end == std::wstring::npos) break;
    start = end + 1;
  }
  return out;
}

// The partial batch is already interactive: number keys and mouse clicks refer
// to its visible positions.  Preserve those positions when the completed batch
// arrives, while still appending newly discovered completed candidates.
inline bool StabilizeCompletedBatch(const std::vector<std::wstring>& currentCandidates,
                                    const std::vector<std::wstring>& currentMeta,
                                    std::vector<std::wstring>* completedCandidates,
                                    std::vector<std::wstring>* completedMeta,
                                    size_t candidateLimit) {
  if (!completedCandidates || !completedMeta || currentCandidates.empty() ||
      completedCandidates->empty() || candidateLimit == 0) {
    return false;
  }
  (void)currentMeta;

  const std::vector<std::wstring> incomingCandidates = std::move(*completedCandidates);
  std::vector<std::wstring> incomingMeta = std::move(*completedMeta);
  incomingMeta.resize(incomingCandidates.size());
  std::vector<bool> incomingUsed(incomingCandidates.size(), false);

  std::vector<std::wstring> stableCandidates;
  std::vector<std::wstring> stableMeta;
  stableCandidates.reserve(std::min(candidateLimit,
                                    currentCandidates.size() + incomingCandidates.size()));
  stableMeta.reserve(stableCandidates.capacity());

  auto alreadyAdded = [&](const std::wstring& phrase) {
    return std::find(stableCandidates.begin(), stableCandidates.end(), phrase) !=
           stableCandidates.end();
  };

  for (size_t i = 0; i < currentCandidates.size() && stableCandidates.size() < candidateLimit;
       ++i) {
    const std::wstring& phrase = currentCandidates[i];
    if (alreadyAdded(phrase)) continue;

    size_t completedIndex = incomingCandidates.size();
    for (size_t j = 0; j < incomingCandidates.size(); ++j) {
      if (!incomingUsed[j] && incomingCandidates[j] == phrase) {
        completedIndex = j;
        break;
      }
    }

    // A partial-only candidate may have been removed by the completed
    // pipeline's filtering/rerank stages. Never turn such a provisional item
    // into a seemingly stable candidate merely to preserve its old slot.
    if (completedIndex >= incomingCandidates.size()) continue;

    stableCandidates.push_back(phrase);
    incomingUsed[completedIndex] = true;
    stableMeta.push_back(incomingMeta[completedIndex]);
  }

  for (size_t i = 0; i < incomingCandidates.size() && stableCandidates.size() < candidateLimit;
       ++i) {
    if (incomingUsed[i] || alreadyAdded(incomingCandidates[i])) continue;
    stableCandidates.push_back(incomingCandidates[i]);
    stableMeta.push_back(incomingMeta[i]);
  }

  const bool changed = stableCandidates != incomingCandidates || stableMeta != incomingMeta;
  *completedCandidates = std::move(stableCandidates);
  *completedMeta = std::move(stableMeta);
  return changed;
}

// Once a candidate snapshot is selectable, its visible indexes are a user
// interface contract: number keys and mouse clicks must keep referring to the
// phrases the user saw. Keep that selectable prefix stable, then append new
// candidates from the completed batch so a one-page partial result can still
// expose later pages after the full lookup finishes.
inline bool FreezeInteractiveBatch(const std::vector<std::wstring>& currentCandidates,
                                   const std::vector<std::wstring>& currentMeta,
                                   std::vector<std::wstring>* completedCandidates,
                                   std::vector<std::wstring>* completedMeta,
                                   size_t candidateLimit) {
  if (!completedCandidates || !completedMeta || currentCandidates.empty() ||
      completedCandidates->empty() || candidateLimit == 0) {
    return false;
  }

  const std::vector<std::wstring> incomingCandidates = std::move(*completedCandidates);
  std::vector<std::wstring> incomingMeta = std::move(*completedMeta);
  incomingMeta.resize(incomingCandidates.size());

  const size_t frozenCount = std::min(candidateLimit, currentCandidates.size());
  std::vector<std::wstring> frozenCandidates(currentCandidates.begin(),
                                             currentCandidates.begin() + frozenCount);
  std::vector<std::wstring> frozenMeta;
  frozenCandidates.reserve(std::min(candidateLimit,
                                    currentCandidates.size() + incomingCandidates.size()));
  frozenMeta.reserve(frozenCandidates.capacity());
  for (size_t i = 0; i < frozenCount; ++i) {
    const auto match = std::find(incomingCandidates.begin(), incomingCandidates.end(),
                                 currentCandidates[i]);
    if (match != incomingCandidates.end()) {
      const size_t index = static_cast<size_t>(match - incomingCandidates.begin());
      frozenMeta.push_back(RemovePartialMetaFlag(incomingMeta[index]));
    } else {
      frozenMeta.push_back(RemovePartialMetaFlag(i < currentMeta.size() ? currentMeta[i]
                                                                          : std::wstring()));
    }
  }

  for (size_t i = 0; i < incomingCandidates.size() && frozenCandidates.size() < candidateLimit;
       ++i) {
    if (std::find(frozenCandidates.begin(), frozenCandidates.end(), incomingCandidates[i]) !=
        frozenCandidates.end()) {
      continue;
    }
    frozenCandidates.push_back(incomingCandidates[i]);
    frozenMeta.push_back(incomingMeta[i]);
  }

  const bool changed = frozenCandidates != incomingCandidates || frozenMeta != incomingMeta;
  *completedCandidates = std::move(frozenCandidates);
  *completedMeta = std::move(frozenMeta);
  return changed;
}

}  // namespace srf_candidate_stability
