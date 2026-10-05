#include <cstdlib>
#include <iostream>
#include <string>
#include <vector>

#include "candidate_layout_policy.h"
#include "candidate_result_stability.h"
#include "clipboard_quick_policy.h"
#include "input_mode_policy.h"
#include "direct_text_conversion.h"
#include "game_input_policy.h"
#include "engine_recovery_policy.h"
#include "input_session.h"
#include <thread>
#include <atomic>

namespace {

void Check(bool condition, const char* message) {
  if (!condition) {
    std::cerr << "FAILED: " << message << '\n';
    std::exit(1);
  }
}

void TestChineseHalfwidth() {
  SrfInputOptions input;
  bool doubleOpen = true, singleOpen = true;
  input.symbolFullwidth = input.numberFullwidth = true;
  Check(SrfConvertDirectText(L"-1A", input, true, true, doubleOpen, singleOpen) == L"\uff0d\uff11\uff21",
        "disabled option preserves fullwidth conversion");
  input.chineseHalfwidth = true;
  std::wstring ascii;
  for (wchar_t ch = 0x20; ch <= 0x7e; ++ch) ascii.push_back(ch);
  Check(SrfConvertDirectText(ascii, input, true, true, doubleOpen, singleOpen) == ascii,
        "halfwidth overrides Chinese punctuation and all fullwidth options");
  Check(doubleOpen && singleOpen, "halfwidth quotes do not alter Chinese quote state");
  Check(SrfConvertDirectText(L"\u4e2d\u6587-", input, true, true, doubleOpen, singleOpen) == L"\u4e2d\u6587-",
        "Chinese text remains intact beside halfwidth hyphen");
  input.chineseHalfwidth = false;
  Check(SrfConvertDirectText(L",", input, true, false, doubleOpen, singleOpen) == L"\uff0c",
        "turning off halfwidth restores Chinese punctuation");
}

void TestGameInputPolicy() {
  using M = SrfGameInputMode;
  Check(!SrfShouldRetryKeyEditSession(true), "failed or partially committed callback is never replayed");
  Check(SrfShouldRetryKeyEditSession(false), "denied sync lock may queue one async callback");
  Check(SrfGameKeepHeldKey(true, true), "held movement repeats stay with game after opening chat");
  Check(!SrfGameKeepHeldKey(true, false), "new down edge can start pinyin");
  Check(!SrfGameKeepHeldKey(false, true), "IME-owned key repeats stay with IME");
  Check(!SrfGameShouldPassThrough(false, M::Manual, false, false), "desktop retains IME");
  Check(SrfGameShouldPassThrough(true, M::Manual, false, false), "game starts in pass-through");
  Check(!SrfGameShouldPassThrough(true, M::Manual, true, false), "explicit chat accepts pinyin");
  Check(SrfGameShouldPassThrough(true, M::Passthrough, true, true), "English game never captures keys");
  Check(!SrfGameShouldPassThrough(true, M::Chinese, false, false), "Chinese game profile");
  Check(SrfGameShouldPassThrough(true, M::AutoText, false, false), "unknown chat surface stays passive");
  Check(!SrfGameShouldPassThrough(true, M::AutoText, false, true), "verified editable surface");
  Check(SrfGameShouldExitChat(true, true, false, true, false, false), "empty Enter/Escape exits");
  Check(!SrfGameShouldExitChat(true, true, true, true, false, false), "Enter selects reading first");
  Check(!SrfGameShouldExitChat(true, true, false, true, true, false), "modified keys retain session");
  Check(!SrfGameShouldExitChat(true, true, false, true, false, true), "held Enter cannot close chat");
}

void TestClipboardQuickPolicy() {
  unsigned int page = 99;
  Check(ParseClipboardQuickPageToken(L"p2", &page) && page == 1, "filtered second batch");
  Check(!ParseClipboardQuickPageToken(L"0", &page), "zero clipboard page rejected");
  Check(!ParseClipboardQuickPageToken(L"4294967297", &page), "overflow cannot wrap to first page");
  Check(!ParseClipboardQuickPageToken(L"p-1", &page), "negative clipboard page rejected");
  Check(!ShouldChangeClipboardQuickBatch(true, 0, 1), "small screen visits remaining three rows first");
  Check(ShouldChangeClipboardQuickBatch(true, 1, 1), "next batch after all eight rows");
  Check(!ShouldChangeClipboardQuickBatch(false, 1, 1), "previous key visits first five rows first");
  Check(ShouldChangeClipboardQuickBatch(false, 0, 1), "previous batch at first visible page");
  Check(ShouldChangeClipboardQuickBatch(true, 0, 0), "large screen visits eight rows in one page");
  Check(!ClipboardQuickBatchHasNext(0, 0), "empty history cannot advance to a phantom batch");
  Check(!ClipboardQuickBatchHasNext(0, 1), "single batch cannot advance past its end");
  Check(ClipboardQuickBatchHasNext(0, 2), "two batches allow one forward transition");
  Check(!ClipboardQuickBatchHasNext(1, 2), "last batch cannot advance past its end");
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

  std::vector<std::wstring> fullCandidates;
  std::vector<std::wstring> fullMeta;
  for (size_t i = 0; i < 300; ++i) {
    fullCandidates.push_back(L"候选" + std::to_wstring(i));
    fullMeta.push_back(L"meta" + std::to_wstring(i));
  }
  std::vector<std::wstring> interactive(fullCandidates.rbegin() + 282,
                                        fullCandidates.rend());
  std::vector<std::wstring> interactiveMeta(interactive.size(), L"partial=1");
  Check(srf_candidate_limits::kFullResult >= fullCandidates.size(),
        "full-result contract covers deep pages");
  Check(FreezeInteractiveBatch(interactive, interactiveMeta, &fullCandidates, &fullMeta,
                               srf_candidate_limits::kFullResult),
        "interactive full batch is frozen");
  Check(fullCandidates.size() == 300,
        "interactive freeze preserves candidates beyond the old 128-row limit");
  Check(fullCandidates.front() == L"候选17" && fullCandidates[17] == L"候选0",
        "interactive prefix remains stable across full completion");
}

}  // namespace

int main() {
  SrfInputSession session;
  const auto pendingKey = session.Capture();
  const auto pendingLookup = session.Capture();
  Check(session.Matches(pendingKey), "current input session accepts its work");
  session.End();
  Check(!session.Matches(pendingKey) && !session.Matches(pendingLookup),
        "same-window input end invalidates both queued keys and candidate results");
  Check(session.Matches(session.Capture()), "new session accepts fresh work");
  SrfEngineRecoveryBudget recovery;
  std::atomic<int> accepted{0};
  std::vector<std::thread> recoveryCallers;
  for (int i = 0; i < 16; ++i) recoveryCallers.emplace_back([&] {
    if (recovery.TryAcquire()) ++accepted;
  });
  for (auto& worker : recoveryCallers) worker.join();
  Check(accepted == 3 && recovery.Exhausted(), "concurrent recovery cannot exceed three attempts");
  for (int i = 0; i < 100; ++i) Check(!recovery.TryAcquire(), "polling cannot reset exhausted recovery");
  recovery.Reset();
  Check(!recovery.Exhausted() && recovery.TryAcquire(), "successful readiness permits a new recovery episode");
  Check(kSrfRecoveryBackoffMs[0] < kSrfRecoveryBackoffMs[1] &&
        kSrfRecoveryBackoffMs[1] < kSrfRecoveryBackoffMs[2], "recovery delays increase");
  TestChineseHalfwidth();
  TestGameInputPolicy();
  TestClipboardQuickPolicy();
  TestInputModePolicy();
  TestCandidateLayoutPolicy();
  TestCandidateResultStability();
  std::cout << "TSF policy tests passed\n";
  return 0;
}
