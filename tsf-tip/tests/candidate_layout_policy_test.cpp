#include "candidate_layout_policy.h"

#include <cassert>

int main() {
  // A horizontal content refresh first keeps clickable geometry stable, then
  // the existing timer applies the smaller natural width.
  assert(ShouldDelayHorizontalCandidateShrink(true, true, true, 640, 420, true));
  assert(!ShouldDelayHorizontalCandidateShrink(false, true, true, 640, 420, true));
  assert(!ShouldDelayHorizontalCandidateShrink(true, false, true, 640, 420, true));
  assert(!ShouldDelayHorizontalCandidateShrink(true, true, false, 640, 420, true));
  assert(!ShouldDelayHorizontalCandidateShrink(true, true, true, 420, 640, true));
  assert(!ShouldDelayHorizontalCandidateShrink(true, true, true, 640, 420, false));

  // Horizontal correction/source metadata must never create a second row.
  assert(!ShouldShowCandidateComment(true, false, true));
  assert(!ShouldShowCandidateComment(true, true, true));
  assert(ShouldShowCandidateComment(false, false, true));
  assert(ShouldShowCandidateComment(false, true, false));
  assert(!ShouldShowCandidateComment(false, false, false));
  return 0;
}
