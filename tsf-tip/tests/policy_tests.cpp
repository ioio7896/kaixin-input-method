#include <cstdlib>
#include <iostream>
#include <string>
#include <vector>

#include "candidate_layout_policy.h"
#include "candidate_result_stability.h"
#include "input_mode_policy.h"

namespace {

void Check(bool condition, const char* message) {
  if (!condition) {
    std::cerr << "FAILED: " << message << '\n';
    std::exit(1);
  }
}

void TestInputModePolicy() {
  const auto chinese = ResolveInitialInputModeState(false, true);
  Check(chinese.imeOpen && chinese.fullShape, "configured Chinese/full-shape defaults");
  const auto ascii = ResolveInitialInputModeState(true, false);
  Check(!ascii.imeOpen && !ascii.fullShape, "configured ASCII/half-shape defaults");
  Check(ClearSystemFullShapeConversion(0b1111, 0b0100) == 0b1011,
        "full-shape conversion flag is cleared");
  Check(ShouldRegisterFullShapeHotkey(true, false), "default full-shape registers hotkey");
  Check(!ShouldRegisterFullShapeHotkey(false, false), "disabled full-shape omits hotkey");
}

void TestCandidateLayoutPolicy() {
  Check(ShouldDelayHorizontalCandidateShrink(true, true, true, 800, 400, true),
        "horizontal content refresh delays shrink");
  Check(!ShouldDelayHorizontalCandidateShrink(false, true, true, 800, 400, true),
        "vertical layout never delays horizontal shrink");
  Check(ShouldShowCandidateComment(false, true, false), "vertical clipboard comment is visible");
  Check(!ShouldShowCandidateComment(true, true, true), "horizontal comment remains one line");
  Check(ShouldAnimateCandidateWindow(false, true, false, false, true),
        "animations run when all policies allow them");
  Check(!ShouldAnimateCandidateWindow(true, true, false, false, true),
        "reduced motion disables animation");
}

void TestCandidateResultStability() {
  using namespace srf_candidate_stability;
  Check(ShouldRetainEmptyCandidateResult(true, true, false, true),
        "transient empty result retains visible candidates");
  Check(RemovePartialMetaFlag(L"source=core\tpartial=1\tselected") ==
            L"source=core\tselected",
        "partial metadata is removed without damaging neighbors");

  const std::vector<std::wstring> current{L"甲", L"乙"};
  const std::vector<std::wstring> currentMeta{L"partial=1", L"partial=1"};
  std::vector<std::wstring> completed{L"乙", L"甲", L"丙"};
  std::vector<std::wstring> completedMeta{L"b", L"a", L"c"};
  Check(StabilizeCompletedBatch(current, currentMeta, &completed, &completedMeta, 3),
        "completed batch is stabilized");
  Check(completed == std::vector<std::wstring>({L"甲", L"乙", L"丙"}),
        "interactive candidate order remains stable");
  Check(completedMeta == std::vector<std::wstring>({L"a", L"b", L"c"}),
        "metadata follows stabilized candidates");
}

}  // namespace

int main() {
  TestInputModePolicy();
  TestCandidateLayoutPolicy();
  TestCandidateResultStability();
  std::cout << "TSF policy tests passed\n";
  return 0;
}
