#pragma once
#include <windows.h>
#include <utility>

// A host can report failure after invoking a synchronous callback. Neither
// asynchronous fallback nor reentrant delivery may execute that callback twice.
class SrfEditSessionOnce {
 public:
  bool Begin() {
    if (executed_) return false;
    executed_ = true;
    return true;
  }
  bool Executed() const { return executed_; }
 private:
  bool executed_ = false;
};

// Ending a composition is cleanup, not another delivery attempt. Keep the
// write result independent so cleanup failure can never replay written text.
struct SrfTsfCommitResult {
  HRESULT write = E_FAIL;
  HRESULT finish = S_OK;
  bool TextWritten() const { return SUCCEEDED(write); }
  bool MayRetryText() const {
    return FAILED(write) && write != HRESULT_FROM_WIN32(ERROR_PARTIAL_COPY) &&
        write != HRESULT_FROM_WIN32(ERROR_CANCELLED) && write != HRESULT_FROM_WIN32(ERROR_PROCESS_ABORTED);
  }
};

template <class Write, class Finish>
SrfTsfCommitResult SrfWriteAndFinishComposition(Write&& write, Finish&& finish) {
  SrfTsfCommitResult result;
  result.write = std::forward<Write>(write)();
  if (result.TextWritten()) result.finish = std::forward<Finish>(finish)();
  return result;
}

class SrfScopedCompositionEnd {
 public:
  explicit SrfScopedCompositionEnd(unsigned& depth) : depth_(depth) { ++depth_; }
  ~SrfScopedCompositionEnd() { --depth_; }
  SrfScopedCompositionEnd(const SrfScopedCompositionEnd&) = delete;
  SrfScopedCompositionEnd& operator=(const SrfScopedCompositionEnd&) = delete;
 private:
  unsigned& depth_;
};

// A comparison failure is unknown, not evidence that the caret left the range.
inline bool SrfSelectionOutsideComposition(HRESULT startHr, LONG start,
                                           HRESULT endHr, LONG end) {
  return SUCCEEDED(startHr) && SUCCEEDED(endHr) && (start < 0 || end > 0);
}
