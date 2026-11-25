#pragma once

#include "background_worker.h"
#include "engine_contract.generated.h"
#include "srf_tip.h"
#include "direct_text_conversion.h"
#include "input_mode_policy.h"
#include "game_rules.generated.h"
#include "input_injection_policy.h"

#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cstdlib>
#include <deque>
#include <cwctype>
#include <cstdio>
#include <cwchar>
#include <dwmapi.h>
#include <filesystem>
#include <mutex>
#include <new>
#include <ole2.h>
#include <inputscope.h>
#include <string>
#include <string_view>
#include <thread>
#include <unordered_set>
#include <vector>

#include "candidate_ui.h"
#include "candidate_overlay_client.h"
#include "candidate_overlay_placement.h"
#include "candidate_result_stability.h"
#include "clipboard_quick_policy.h"
#include "composition_sink.h"
#include "guids.h"
#include "ime_config.h"
#include "key_sink.h"
#include "pinyin_stub.h"
#include "privacy_sensitive_context.h"
#include "thread_mgr_sink.h"
#include "text_edit_sink.h"
#include "tsf_edit_policy.h"
#include "diagnostic_privacy.h"
#include <functional>

extern LONG g_cSrfTipObjects;

bool SrfTsfDebugTraceEnabled();
void SrfTsfDebugLog(const wchar_t* msg);
void SrfTsfDiagnosticLog(const wchar_t* tag, const wchar_t* msg);
void SrfTsfPerfLog(const wchar_t* tag, const wchar_t* msg);
bool SrfTsfPerfLogEnabled();

namespace srf_tip_detail {

enum class SrfTsfLogLevel {
  Off = 0,
  Error = 1,
  Basic = 2,
  Perf = 3,
  Verbose = 4,
};

bool IsContinuousTranslateFlagPresent();
bool LaunchCandidateTranslation(const std::wstring& text, HWND targetHwnd, uint64_t focusGeneration,
                                const RECT* caretRect = nullptr,
                                const wchar_t* targetOverride = nullptr,
                                const wchar_t* originOverride = nullptr, bool ephemeral = false,
                                bool allowClipboardFallback = false);
inline constexpr DWORD kRustModeFuzzy = srf_engine_contract::MODE_FUZZY_PINYIN;
inline constexpr DWORD kRustModeDouble = srf_engine_contract::MODE_DOUBLE_PINYIN;
inline constexpr DWORD kRustModeJianpin = srf_engine_contract::MODE_JIANPIN;
inline constexpr DWORD kRustModeVAssist = srf_engine_contract::MODE_V_ASSIST;
inline constexpr DWORD kRustModeUMode = srf_engine_contract::MODE_U_MODE;
inline constexpr DWORD kRustModeDateAutoFormat = srf_engine_contract::MODE_DATE_AUTO_FORMAT;
inline constexpr DWORD kRustModeClipboardBackground =
    srf_engine_contract::MODE_CLIPBOARD_BACKGROUND;
inline constexpr DWORD kRustModeMixedPinyin = srf_engine_contract::MODE_MIXED_PINYIN;
inline constexpr DWORD kRustModeMixedPinyinAggressive =
    srf_engine_contract::MODE_MIXED_PINYIN_AGGRESSIVE;
inline constexpr DWORD kRustModeTraditionalOutput = srf_engine_contract::MODE_TRADITIONAL_OUTPUT;
inline constexpr DWORD kRustModeEnglishWordInput = srf_engine_contract::MODE_ENGLISH_WORD_INPUT;
inline constexpr DWORD kRustModeUserPhraseComposeActive =
    srf_engine_contract::MODE_USER_PHRASE_COMPOSE_ACTIVE;
inline constexpr DWORD kRustModeClipboardDisabled = srf_engine_contract::MODE_CLIPBOARD_DISABLED;
inline constexpr DWORD kRustModeSymbolToolbox = srf_engine_contract::MODE_SYMBOL_TOOLBOX;
inline constexpr DWORD kRustModeEmojiInput = srf_engine_contract::MODE_EMOJI_INPUT;
inline constexpr DWORD kRustModeLearningAggressive = srf_engine_contract::MODE_LEARNING_AGGRESSIVE;
inline constexpr DWORD kRustModeLearningConservative =
    srf_engine_contract::MODE_LEARNING_CONSERVATIVE;
inline constexpr wchar_t kStateRegPath[] = L"Software\\kaixin\\State";
inline constexpr wchar_t kStateAsciiValue[] = L"AsciiMode";
inline constexpr wchar_t kStatePureGameModeValue[] = L"PureGameMode";
inline constexpr ULONGLONG kImeTogglePreservedKeyGuardMs = 600;
bool SrfTsfLogEnabled(SrfTsfLogLevel level);
void DebugLogPerfMs(const wchar_t* stage, ULONGLONG startTick);
const wchar_t* EngineStateName(SrfEngineState state);
void NotifyTrayAsciiStateChanged(bool asciiMode);
std::wstring ProcessNameForWindow(HWND hwnd, DWORD* processIdOut = nullptr);
void PublishTrayInputStatus(bool asciiMode, bool fullShape, bool chinesePunctuation,
                            const std::wstring& modeSource);
std::wstring ShortenForLog(const std::wstring& text, size_t maxUnits = 48);
std::wstring FormatRectForLog(const RECT& rect);
std::wstring FormatPointerForLog(const void* ptr);
bool HasCtrlOrAltDown();
bool HasCtrlOrShiftDown();
bool IsLetterVk(UINT vk);
bool IsVkShift(UINT vk);
bool IsDigitVk(UINT vk);
wchar_t DigitCharFromVk(UINT vk);
int CandidateNumberIndexFromVk(UINT vk);
void TrimWstringInPlace(std::wstring& s);
void MaybeInsertPredictedPhraseCandidates(const std::wstring& reading,
                                          std::vector<std::wstring>* candidates,
                                          std::vector<CandidateMetaParts>* candidateMeta);
std::wstring CandidateSourceComment(const CandidateMetaParts& meta);
void PrefixCandidateDisplayText(std::wstring* text, const CandidateMetaParts& meta,
                                const SrfUIStyle& style);
void AppendCommentPart(std::wstring* out, const std::wstring& part);
bool IsFunctionKeyTokenPrefix(const std::wstring& text);
bool IsValidFunctionKeyToken(const std::wstring& text);
std::wstring LowerAscii(std::wstring text);
bool TryParseFunctionKeyToken(const std::wstring& text, UINT* outVk);
bool SendVirtualKeyTap(UINT vk);
HRESULT SetUnicodeClipboardTextForPaste(const std::wstring& text, DWORD* out_sequence = nullptr);
HRESULT PasteUnicodeTextViaClipboard(const std::wstring& text, CSrfTip* tip,
                                     const std::function<bool()>& valid);
HRESULT SendUnicodeTextInput(const std::wstring& text);
bool IsNumpadPrintableVk(UINT vk);
bool IsOemPrintableVk(UINT vk);
bool IsPrintableDirectVk(UINT vk);
bool ParseClipboardQuickReading(const std::wstring& reading, UINT* outPage,
                                std::wstring* outFilter);
std::wstring BuildClipboardQuickReading(UINT page, const std::wstring& filter);
bool IsDirectInsertPreferredChar(wchar_t ch);
std::wstring FallbackPrintableText(UINT vk, bool shiftDown, bool capsLockOn);
TF_DISPLAYATTRIBUTE DefaultDisplayAttribute();
bool IsUsableRect(const RECT& rect);
HWND RootWindowForPlacement(HWND hwnd);
HWND ResolveContextWindow(ITfContext* context);
bool TryGetTextExtRect(TfEditCookie ec, ITfContext* context, ITfRange* range, RECT* rect);
bool TryGetGuiCaretRect(RECT* rect);
bool TryGetMouseRect(HWND targetHwnd, RECT* rect);
bool TryGetWindowBottomLeftRect(HWND hwnd, RECT* rect);
bool TryGetScreenSafeRect(HWND targetHwnd, RECT* rect);
void DebugLogCandidateAnchorSource(const wchar_t* source, const RECT& rect);
bool WildcardMatchNoCase(const std::wstring& pattern, const std::wstring& value);
bool IsBuiltinStrictFocusProcessName(const std::wstring& appName);
std::wstring WindowClassName(HWND hwnd);
bool CurrentProcessAppContainerState(bool* known);
std::wstring CurrentProcessIntegrityLabel();
std::wstring SanitizeDiagnosticValue(std::wstring value);
bool IsBuiltinGameWindowClass(const std::wstring& className);
bool IsBuiltinGameProcessName(const std::wstring& appName);
bool IsBuiltinAsciiOnlyProcessName(const std::wstring& appName);
bool IsConfiguredGameProcessName(const SrfConfig& config, const std::wstring& appName);
bool MatchesConfiguredProcessList(const std::vector<std::wstring>& patterns,
                                  const std::wstring& appName);
bool IsFullscreenForegroundWindow(HWND hwnd);
std::filesystem::path LocalDataDir();
void AppendUtf8FileLine(const std::filesystem::path& path, const std::wstring& line);
std::filesystem::path ModuleDirFromAddress(const void* address);
std::filesystem::path FindTrayHelper(const std::filesystem::path& moduleDir);
HRESULT LaunchSettingsHelper(HWND parent);
void EnsureTrayHelperRunning();
UINT PageStartFromMetrics(const CandidatePageLayoutMetrics& metrics, UINT page);
UINT PageEndExclusiveFromMetrics(const CandidatePageLayoutMetrics& metrics, UINT page,
                                 UINT totalItems);
UINT PageForIndexFromMetrics(const CandidatePageLayoutMetrics& metrics, UINT index,
                             UINT totalItems);
HRESULT GetInsertionRange(TfEditCookie ec, ITfContext* pic, ITfRange** ppRange);
HRESULT SetCompositionDisplay(TfEditCookie ec, ITfComposition* composition,
                              const std::wstring& text);
HRESULT CollapseSelectionToRangeOffset(TfEditCookie ec, ITfContext* pic, ITfRange* range,
                                       LONG offset);
HRESULT CollapseSelectionToRangeEnd(TfEditCookie ec, ITfContext* pic, ITfRange* range);

}  // namespace srf_tip_detail
