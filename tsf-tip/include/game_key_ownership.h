#pragma once
#include <array>
#include <cstddef>
#include <cstdint>

// Logical cancellation must not wait for a host to release queued COM edits.
// An old callback cannot decrement the new chat session's pending count.
class SrfPendingKeyEdits {
 public:
  std::uint64_t Begin(std::uint64_t now) {
    if (count_++ == 0) first_ = now;
    return generation_;
  }
  void Complete(std::uint64_t generation) {
    if (generation != generation_ || !count_) return;
    if (--count_ == 0) first_ = 0;
  }
  void Cancel() { ++generation_; count_ = 0; first_ = 0; }
  bool Pending() const { return count_ != 0; }
  bool TimedOut(std::uint64_t now) const { return count_ && now >= first_ && now - first_ > 1000; }
 private:
  std::uint64_t generation_ = 1;
  std::uint64_t first_ = 0;
  unsigned count_ = 0;
};

class SrfGameKeyOwnership {
 public:
  void Reset() { down_.fill(false); released_.fill(false); known_.fill(false); }
  bool KeepDown(unsigned key, bool repeated, bool game = false) const {
    return key < down_.size() && repeated && (down_[key] || (game && !known_[key]));
  }
  bool PassDown(unsigned key, bool repeated, bool game) const {
    return key < down_.size() && (down_[key] || (repeated && game && !known_[key]));
  }
  bool ConsumeRepeat(unsigned key, bool repeated, bool game) const {
    return key < down_.size() && repeated && game && known_[key] && !down_[key];
  }
  bool PassRelease(unsigned key) const {
    return key < down_.size() && (down_[key] || released_[key]);
  }
  void ObserveDown(unsigned key, bool repeated, bool passed) {
    if (key >= down_.size()) return;
    released_[key] = false;
    if (!repeated || !known_[key]) { down_[key] = passed; known_[key] = true; }
  }
  void ObserveUp(unsigned key) {
    if (key >= down_.size()) return;
    released_[key] = down_[key];
    down_[key] = false;
    known_[key] = false;
  }
 private:
  std::array<bool, 256> down_ = {};
  std::array<bool, 256> released_ = {};
  std::array<bool, 256> known_ = {};
};
