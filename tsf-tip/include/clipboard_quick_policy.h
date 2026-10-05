#pragma once

#include <limits>
#include <string>

inline bool ParseClipboardQuickPageToken(const std::wstring& token, unsigned int* outPage) {
  if (!outPage || token.empty()) return false;
  size_t pos = (token[0] == L'p' || token[0] == L'P') ? 1 : 0;
  if (pos >= token.size()) return false;
  unsigned int page = 0;
  for (; pos < token.size(); ++pos) {
    const wchar_t ch = token[pos];
    if (ch < L'0' || ch > L'9') return false;
    const unsigned int digit = static_cast<unsigned int>(ch - L'0');
    if (page > (std::numeric_limits<unsigned int>::max() - digit) / 10) return false;
    page = page * 10 + digit;
  }
  if (page == 0) return false;
  *outPage = page - 1;
  return true;
}

// A quick batch contains eight entries, but its window may show only five.
// Exhaust the visible pages before asking the engine for another batch.
inline bool ShouldChangeClipboardQuickBatch(bool next, unsigned int localPage,
                                            unsigned int lastLocalPage) {
  return next ? localPage >= lastLocalPage : localPage == 0;
}

inline bool ClipboardQuickBatchHasNext(unsigned int page, unsigned int totalPages) {
  return page < totalPages && totalPages - page > 1;
}
