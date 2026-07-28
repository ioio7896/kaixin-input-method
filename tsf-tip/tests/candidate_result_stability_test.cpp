#include "candidate_result_stability.h"

#include <cassert>
#include <string>
#include <vector>

int main() {
  using srf_candidate_stability::RemovePartialMetaFlag;
  using srf_candidate_stability::CanRefreshCompletedPartialSnapshot;
  using srf_candidate_stability::CompletedBatchKeepsTop;
  using srf_candidate_stability::FreezeInteractiveBatch;
  using srf_candidate_stability::ShouldRetainEmptyCandidateResult;
  using srf_candidate_stability::StabilizeCompletedBatch;

  assert(RemovePartialMetaFlag(L"source=system\tpartial=1\t12.00") ==
         L"source=system\t12.00");
  assert(CompletedBatchKeepsTop(L"旧首选", L"旧首选"));
  assert(!CompletedBatchKeepsTop(L"旧首选", L"新首选"));
  assert(ShouldRetainEmptyCandidateResult(true, true, false, true));
  assert(!ShouldRetainEmptyCandidateResult(true, true, false, false));
  assert(CanRefreshCompletedPartialSnapshot(true, true, 7, 7));
  assert(!CanRefreshCompletedPartialSnapshot(true, true, 8, 7));
  assert(!CanRefreshCompletedPartialSnapshot(true, false, 7, 7));
  assert(!CanRefreshCompletedPartialSnapshot(false, true, 7, 7));

  std::vector<std::wstring> current = {L"旧一", L"旧二", L"旧三"};
  std::vector<std::wstring> currentMeta = {
      L"source=system\tpartial=1", L"source=system", L"source=system"};
  std::vector<std::wstring> completed = {L"新首选", L"旧三", L"旧一", L"新候选"};
  std::vector<std::wstring> completedMeta = {
      L"new-top", L"full-three", L"full-one", L"new-tail"};

  assert(StabilizeCompletedBatch(current, currentMeta, &completed, &completedMeta, 100));
  const std::vector<std::wstring> expected = {L"旧一", L"旧三", L"新首选", L"新候选"};
  assert(completed == expected);
  assert(completedMeta[0] == L"full-one");
  assert(completedMeta[1] == L"full-three");

  std::vector<std::wstring> limitedCompleted = {L"新首选", L"旧一"};
  std::vector<std::wstring> limitedMeta = {L"new", L"old"};
  assert(StabilizeCompletedBatch(current, currentMeta, &limitedCompleted, &limitedMeta, 3));
  assert((limitedCompleted == std::vector<std::wstring>{L"旧一", L"新首选"}));
  assert(limitedCompleted.size() == limitedMeta.size());

  // A completed lookup with a new top and a removed provisional candidate
  // must not change what the already-visible number keys mean. Newly found
  // candidates are appended so later candidate pages remain available.
  std::vector<std::wstring> reordered = {L"新首选", L"旧三", L"旧一", L"新候选"};
  std::vector<std::wstring> reorderedMeta = {L"new-top", L"full-three", L"full-one",
                                             L"new-tail"};
  assert(FreezeInteractiveBatch(current, currentMeta, &reordered, &reorderedMeta, 100));
  assert((reordered ==
          std::vector<std::wstring>{L"旧一", L"旧二", L"旧三", L"新首选", L"新候选"}));
  assert(reorderedMeta[0] == L"full-one");
  assert(reorderedMeta[1] == L"source=system");
  assert(reorderedMeta[2] == L"full-three");
  assert(reorderedMeta[3] == L"new-top");
  assert(reorderedMeta[4] == L"new-tail");

  // Regression: the interactive partial batch is exactly one five-item page.
  // Completing the lookup must preserve that page and append enough unique
  // candidates to make PageDown available.
  std::vector<std::wstring> partialPage = {L"一", L"二", L"三", L"四", L"五"};
  std::vector<std::wstring> partialPageMeta(partialPage.size(), L"source=system\tpartial=1");
  std::vector<std::wstring> fullBatch = {
      L"新首", L"二", L"一", L"六", L"七", L"八", L"九", L"十", L"十一", L"十二"};
  std::vector<std::wstring> fullBatchMeta(fullBatch.size(), L"source=system");
  assert(FreezeInteractiveBatch(partialPage, partialPageMeta, &fullBatch, &fullBatchMeta, 128));
  assert(fullBatch.size() == 13);
  assert(std::equal(partialPage.begin(), partialPage.end(), fullBatch.begin()));
  assert(fullBatch[5] == L"新首");
  assert(fullBatchMeta.size() == fullBatch.size());
  assert(fullBatchMeta[0] == L"source=system");

  std::vector<std::wstring> same = current;
  const std::vector<std::wstring> currentSameMeta = {L"source=system", L"source=system",
                                                      L"source=system"};
  std::vector<std::wstring> sameMeta = {L"source=system", L"source=system", L"source=system"};
  assert(!FreezeInteractiveBatch(current, currentSameMeta, &same, &sameMeta, 100));
  return 0;
}
