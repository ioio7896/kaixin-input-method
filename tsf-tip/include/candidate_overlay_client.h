#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <cstdint>
#include <mutex>
#include <string>
#include <vector>

#include "candidate_overlay_protocol.h"

class CExternalCandidateOverlayClient {
 public:
  CExternalCandidateOverlayClient() = default;
  ~CExternalCandidateOverlayClient();
  CExternalCandidateOverlayClient(const CExternalCandidateOverlayClient&) = delete;
  CExternalCandidateOverlayClient& operator=(const CExternalCandidateOverlayClient&) = delete;

  // Starts the helper without waiting for it. Calling this while applying a
  // game profile keeps process startup off the first candidate-key path.
  void Prewarm();

  // Returns true only when the independent helper accepted the snapshot. The
  // On the initial handoff, the caller keeps the in-process window until
  // QueryStatus reports that a sequence has actually been applied. Once the
  // same owner is visible, later updates may keep that previous external frame
  // during the short asynchronous apply window.
  bool Show(HWND senderHwnd, std::uint64_t ownerId,
            const SrfCandidateOverlaySnapshot& snapshot,
            std::uint64_t* acceptedSequence = nullptr);
  SrfCandidateOverlayStatus QueryStatus(HWND senderHwnd,
                                        std::uint64_t ownerId,
                                        std::uint64_t sequence);
  void Hide(HWND senderHwnd, std::uint64_t ownerId, HWND targetHwnd,
            DWORD targetProcessId,
            std::uint64_t focusGeneration);
  void Shutdown();

 private:
  HWND FindHelperWindowLocked();
  bool StartHelperIfNeededLocked();
  bool SendLocked(HWND senderHwnd, SrfCandidateOverlaySnapshot snapshot,
                  std::uint64_t* acceptedSequence = nullptr);
  bool IsHelperProcessAliveLocked() const;
  bool IsExpectedHelperWindowLocked(HWND hwnd) const;
  void ResetHelperLocked(bool requestClose);
  void TerminateHelperLocked();

  std::mutex mutex_;
  HWND helperWindow_ = nullptr;
  HANDLE helperProcess_ = nullptr;
  DWORD helperProcessId_ = 0;
  std::wstring controlWindowToken_;
  SrfCandidateOverlayAuthSecret authSecret_ = {};
  std::uint64_t activeOwnerId_ = 0;
  ULONGLONG helperLaunchTick_ = 0;
  ULONGLONG lastLaunchAttemptTick_ = 0;
  std::uint64_t nextSequence_ = 0;
};

CExternalCandidateOverlayClient& GetExternalCandidateOverlayClient();
