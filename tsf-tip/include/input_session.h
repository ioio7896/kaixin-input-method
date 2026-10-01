#pragma once
#include <cstdint>
// Every input-scoped asynchronous task captures this epoch. Physical focus
// identity stays separate, so ending input in the same window invalidates it.
class SrfInputSession {
 public:
  std::uint64_t Capture() const { return m_epoch; }
  bool Matches(std::uint64_t epoch) const { return epoch == m_epoch; }
  void End() { ++m_epoch; }
 private:
  std::uint64_t m_epoch = 1;
};
