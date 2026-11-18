#pragma once
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <cstdint>

struct SrfGameFocusEvidence {
  HWND target = nullptr;
  HWND nativeFocus = nullptr;
  DWORD processId = 0;
  std::uint64_t context = 0;
  std::uint64_t generation = 0;
  std::uint64_t element = 0;
  ULONGLONG checked = 0;
  ULONGLONG stableSince = 0;
  std::uint64_t focusRevision = 0;
  bool editable = false;
  bool needsStability = false;
  bool insertProbePerformed = false;
  bool insertAtSelectionAvailable = false;
  bool insertQueryAccepted = false;
  RECT bounds = {};
};

// Only copies cached evidence and queues work. No COM/provider calls on the key thread.
bool SrfPollGameFocusEvidence(const SrfGameFocusEvidence& request, SrfGameFocusEvidence* result);
