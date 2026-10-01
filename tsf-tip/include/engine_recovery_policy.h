#pragma once
#include <array>
#include <atomic>
#include <cstddef>

// Shared by every automatic recovery entry point, reset only after readiness
// or an explicit change of the recovery setting.
inline constexpr std::array<unsigned long, 3> kSrfRecoveryBackoffMs{1500, 3000, 6000};
class SrfEngineRecoveryBudget {
 public:
  bool Exhausted() const { return m_attempts.load() >= kSrfRecoveryBackoffMs.size(); }
  bool TryAcquire() {
    auto current = m_attempts.load();
    while (current < kSrfRecoveryBackoffMs.size()) {
      if (m_attempts.compare_exchange_weak(current, current + 1)) return true;
    }
    return false;
  }
  void Reset() { m_attempts.store(0); }
 private:
  std::atomic<std::size_t> m_attempts{0};
};
