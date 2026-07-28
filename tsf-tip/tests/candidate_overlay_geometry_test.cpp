#include "candidate_overlay_placement.h"

int main() {
  const RECT rightMonitor = {1920, 0, 4480, 1440};
  const RECT leftMonitor = {-2560, 0, 0, 1440};
  const RECT upperMonitor = {0, -1440, 2560, 0};

  if (!CandidateOverlayRectCoversMonitor(rightMonitor, rightMonitor)) return 1;
  if (!CandidateOverlayRectCoversMonitor(leftMonitor, leftMonitor)) return 2;
  if (!CandidateOverlayRectCoversMonitor(upperMonitor, upperMonitor)) return 3;

  const RECT insetTwo = {1922, 2, 4478, 1438};
  const RECT insetThree = {1923, 3, 4477, 1437};
  if (!CandidateOverlayRectCoversMonitor(insetTwo, rightMonitor)) return 4;
  if (CandidateOverlayRectCoversMonitor(insetThree, rightMonitor)) return 5;

  const RECT primaryOnly = {0, 0, 1920, 1080};
  if (CandidateOverlayRectCoversMonitor(primaryOnly, leftMonitor)) return 6;
  const RECT workAreaOnly = {1920, 0, 4480, 1400};
  if (CandidateOverlayRectCoversMonitor(workAreaOnly, rightMonitor)) return 7;

  const RECT empty = {0, 0, 0, 0};
  if (CandidateOverlayRectCoversMonitor(empty, rightMonitor) ||
      CandidateOverlayRectCoversMonitor(rightMonitor, empty) ||
      CandidateOverlayRectCoversMonitor(rightMonitor, rightMonitor, -1)) {
    return 8;
  }
  return 0;
}
