#pragma once

#include <string>
#include <tuple>

struct CandidateMetaParts {
  std::wstring display;
  std::wstring annotation;
  std::wstring score;
  std::wstring correctedReading;
  bool noLearn = false;
  bool clipboardQuick = false;
  std::wstring clipboardId;
  std::wstring clipboardSource;
  std::wstring clipboardTime;
  std::wstring clipboardType;
  std::wstring clipboardFilter;
  unsigned clipboardPage = 1;
  unsigned clipboardPages = 1;
  bool clipboardPinned = false;
  bool forceVerticalLayout = false;
  bool partialResult = false;
  bool correctionCandidate = false;
  bool prefixPlaceholder = false;
  bool pinnedExactInput = false;
  bool userCandidate = false;
  std::wstring userSourceLabel;
  bool extCandidate = false;

  bool operator==(const CandidateMetaParts& other) const {
    return std::tie(display, annotation, score, correctedReading, noLearn, clipboardQuick,
                    clipboardId, clipboardSource, clipboardTime, clipboardType, clipboardFilter,
                    clipboardPage, clipboardPages, clipboardPinned, forceVerticalLayout,
                    partialResult, correctionCandidate, prefixPlaceholder, pinnedExactInput,
                    userCandidate, userSourceLabel, extCandidate) ==
           std::tie(other.display, other.annotation, other.score, other.correctedReading,
                    other.noLearn, other.clipboardQuick, other.clipboardId, other.clipboardSource,
                    other.clipboardTime, other.clipboardType, other.clipboardFilter,
                    other.clipboardPage, other.clipboardPages, other.clipboardPinned,
                    other.forceVerticalLayout, other.partialResult, other.correctionCandidate,
                    other.prefixPlaceholder, other.pinnedExactInput, other.userCandidate,
                    other.userSourceLabel, other.extCandidate);
  }
};
