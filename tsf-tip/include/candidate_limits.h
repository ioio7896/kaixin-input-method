#pragma once

#include <cstddef>

namespace srf_candidate_limits {

// Keep in sync with Rust LOOKUP_FULL_MAX_CANDIDATES. This covers the largest
// compiled single-syllable character pool, including its uncommon tail.
inline constexpr std::size_t kFullResult = 1024;

}  // namespace srf_candidate_limits
