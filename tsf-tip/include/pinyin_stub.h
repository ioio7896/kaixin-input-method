#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <string>
#include <vector>

enum class SrfEngineState : unsigned char {
  Idle = 0,
  Loading,
  Ready,
  Failed,
};

enum class SrfLookupCandidatesStatus : unsigned char {
  Ok = 0,
  Empty,
  EngineNotReady,
  BridgeBusy,
  EnsureFailed,
  Superseded,
  RemoteBusy,
  TransientFailure,
  Failed,
  BackendNotConnected,
};

enum SrfLearnCommitFlags : unsigned long {
  kSrfLearnCommitDefault = 0,
  kSrfLearnCommitWeak = 1u << 0,
  kSrfLearnCommitComposedPhrase = 1u << 1,
};

enum SrfCandidateAction : unsigned long {
  kSrfCandidateActionRemoveUserPhrase = 1,
  kSrfCandidateActionBlockPhrase = 2,
  kSrfCandidateActionUnblockPhrase = 3,
};

bool SrfTip_InitializeEngine();
void SrfTip_SetRetryOnFailureEnabled(bool enabled);
void SrfTip_WarmupEngineAsync();
SrfEngineState SrfTip_GetEngineState();
std::wstring SrfTip_GetEngineFailureDetail();
void SrfTip_SetEngineModeFlags(unsigned long flags);
void SrfTip_ClearLookupCache();
void SrfTip_LearnCommit(const std::wstring& reading, const std::wstring& committedText);
void SrfTip_LearnCommitEx(const std::wstring& reading, const std::wstring& committedText,
                          unsigned long flags);
// Queue learning and post completionMessage to completionWindow only after the
// engine has answered. WPARAM is the returned request id; LPARAM is non-zero
// on success. Returning zero means the request was not queued.
unsigned long long SrfTip_LearnCommitExWithCompletion(
    const std::wstring& reading, const std::wstring& committedText, unsigned long flags,
    HWND completionWindow, UINT completionMessage);
/// 学习一次纠错对：用户从纠错候选上屏后，把 raw reading 与 corrected reading 的对应关系写入用户词库。
void SrfTip_LearnCorrection(const std::wstring& rawReading,
                            const std::wstring& correctedReading,
                            const std::wstring& committedText);
void SrfTip_LearnSelectionFeedback(const std::wstring& reading,
                                   const std::wstring& committedText,
                                   unsigned long selectedIndex,
                                   unsigned long page,
                                   const std::vector<std::wstring>& skippedCandidates = {});
void SrfTip_ResetLearningContext();
bool SrfTip_SetCandidatePin(const std::wstring& reading, const std::wstring& committedText,
                          bool pinned);
void SrfTip_ApplyCandidateAction(const std::wstring& reading,
                                 const std::wstring& committedText,
                                 SrfCandidateAction action);
void SrfTip_RecordClipboardText(const std::wstring& text);
bool SrfTip_ResolveClipboardText(const std::wstring& id, std::wstring* text);
/// 解析剪贴板条目文本；命中本地缓存时零管道往返（供提交路径使用）。
bool SrfTip_ResolveClipboardTextCached(const std::wstring& id, std::wstring* text);
/// 后台预取剪贴板条目文本到本地缓存（vvu 候选列表构建后调用，幂等且合并）。
void SrfTip_PrefetchClipboardTexts(const std::vector<std::wstring>& ids);
unsigned long long SrfTip_NextLookupRequestId();
// Best-effort, non-blocking publication of a newer request. The helper only
// cancels an active lookup from this TIP process whose request id is older.
void SrfTip_CancelPendingLookupBefore(unsigned long long requestId);
SrfLookupCandidatesStatus SrfTip_LookupCandidates(
    const std::wstring& reading, std::vector<std::wstring>& candidates,
    std::vector<std::wstring>* metaScores = nullptr, unsigned long long requestId = 0,
    bool fullResult = false, bool* hasMore = nullptr);
bool SrfTip_TryGetCachedLookupCandidates(const std::wstring& reading,
                                         std::vector<std::wstring>& candidates,
                                         std::vector<std::wstring>* metaScores = nullptr,
                                         bool* hasMore = nullptr);
void SrfTip_PrewarmSingleLetterLookupCacheAsync();

/// 音节边界 UTF-16 偏移（含首尾）；返回写入个数。DLL 无导出时返回 0。
size_t SrfTip_SyllableBoundaryOffsetsUtf16(const wchar_t* text, size_t unitCount, uint32_t* out,
                                           size_t cap);
