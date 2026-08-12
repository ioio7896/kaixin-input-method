#include "pinyin_stub.h"

#include <windows.h>

#include <sddl.h>
#include <wincrypt.h>

#include <algorithm>
#include <array>
#include <atomic>
#include <cctype>
#include <cstdio>
#include <deque>
#include <cwctype>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <mutex>
#include <string>
#include <thread>
#include <utility>
#include <vector>

extern "C" void SrfTip_BackgroundWorkerAddRef();
extern "C" void SrfTip_BackgroundWorkerRelease();

#ifndef SRF_APP_VERSION
#define SRF_APP_VERSION "dev"
#endif

#ifndef SRF_GIT_COMMIT
#define SRF_GIT_COMMIT "unknown"
#endif

#ifndef SRF_GIT_DIRTY
#define SRF_GIT_DIRTY 0
#endif

namespace {

constexpr size_t kRustRows = 128;
constexpr size_t kRustRowTextUnits = 512;
constexpr size_t kRustRowMetaUnits = 512;
constexpr size_t kRustRowWidth = kRustRowTextUnits + kRustRowMetaUnits;
constexpr uint32_t kEnginePipeMagic = 0x31504653;  // "SFP1"
constexpr uint16_t kEnginePipeVersion = 5;
constexpr DWORD kEnginePipeRetrySleepMs = 60;
// Init runs from warmup/retry workers, but the code still lives inside host
// processes. Keep it short so a broken helper or stale registration cannot
// make apps such as WPS look hung to Windows.
constexpr DWORD kEnginePipeInitTimeoutMs = 1800;
// Lookup runs on the typing path, so we still want a bounded wait. In practice
// the Rust helper can exceed 1 s on cold disks, under AV scanning, or while
// another client thread is finishing a heavier lookup. Keep the first wait
// reasonably short, but allow a more forgiving retry window before we tear
// down the bridge and leave the user with no candidates.
constexpr DWORD kEnginePipeLookupTimeoutMs = 120;
constexpr DWORD kEnginePipeLookupRetryTimeoutMs = 650;
constexpr DWORD kEnginePipeLearnTimeoutMs = 750;
constexpr DWORD kAsyncLearnReadyWaitMs = 1800;
constexpr DWORD kEnginePipeHealthTimeoutMs = 500;
constexpr DWORD kEnginePipeClipboardTimeoutMs = 750;
constexpr DWORD kEnginePipeSyllableTimeoutMs = 50;
constexpr DWORD kEnginePipeShutdownTimeoutMs = 250;
constexpr DWORD kEnginePipeCancelTimeoutMs = 35;
constexpr DWORD kEnginePipeStartupReadyTimeoutMs = 1200;
constexpr wchar_t kDefaultEnginePipeName[] = LR"(\\.\pipe\KaixinInput_Engine_V5)";
constexpr wchar_t kDefaultEngineMutexName[] = L"Local\\KaixinInput_Engine_Mutex_V5";
constexpr wchar_t kEnginePipePrefix[] = LR"(\\.\pipe\KaixinInput_Engine_V5_)";
constexpr wchar_t kEngineMutexPrefix[] = L"Local\\KaixinInput_Engine_Mutex_V5_";
constexpr std::array<DWORD, 3> kRetryBackoffMs = {1500, 3000, 6000};
constexpr DWORD kIdleKeepaliveTimeoutMs = 5 * 60 * 1000;
constexpr DWORD kIdleCheckIntervalMs = 60 * 1000;
constexpr DWORD kEngineWatchdogIntervalMs = 30 * 1000;
constexpr DWORD kInstallMaintenanceMaxAgeMs = 30 * 60 * 1000;
constexpr DWORD kFailedStateWarmupCooldownMs = 1500;
constexpr unsigned kLookupTimeoutRestartThreshold = 3;
constexpr DWORD kLookupTimeoutRestartCooldownMs = 10000;
constexpr SIZE_T kEngineHelperMemoryLimitBytes = 2ull * 1024ull * 1024ull * 1024ull;
constexpr size_t kMaxBridgeInputUnits = 256;
constexpr size_t kMaxLearnPhraseUnits = 512;
constexpr size_t kMaxSelectionFeedbackSkipped = 64;
constexpr size_t kMaxClipboardTextUnits = 20000;
constexpr size_t kMaxLexiconPathUnits = 4096;
constexpr uint32_t kRustModeTraditionalOutput = 0x0200;
constexpr size_t kEnginePipeResponseHeaderBytes = 16;
constexpr size_t kEnginePipeMaxResponseBytes =
    kEnginePipeResponseHeaderBytes + 4 + kRustRows * kRustRowWidth * sizeof(uint16_t);
constexpr int kRustFfiPanicRc = -100;
constexpr wchar_t kAppPathName[] = L"kaixin";
constexpr wchar_t kStateRegPath[] = L"Software\\kaixin\\State";
constexpr wchar_t kStateInstallMaintenanceValue[] = L"InstallMaintenance";
constexpr wchar_t kStateInstallMaintenanceTickValue[] = L"InstallMaintenanceTick";
constexpr wchar_t kStateEngineStateValue[] = L"EngineState";
constexpr wchar_t kStateLastEngineRecoveryReasonValue[] = L"LastEngineRecoveryReason";
constexpr wchar_t kStateLastEngineRecoveryTimeValue[] = L"LastEngineRecoveryTime";
constexpr wchar_t kEngineCapabilityFileName[] = L"engine_capability.dat";
constexpr char kEngineCapabilityMagic[] = "KXIPC-DPAPI-1\n";

enum class EnginePipeCommand : uint16_t {
  Init = 1,
  Lookup = 2,
  Learn = 3,
  SyllableBounds = 4,
  RecordClipboard = 5,
  SetCandidatePin = 6,
  Health = 7,
  ResolveClipboard = 8,
  Shutdown = 9,
  LearnCorrection = 10,
  LearnSelectionFeedback = 11,
  CandidateAction = 12,
  ResetLearningContext = 13,
  CancelLookup = 14,
};

enum class EngineBackend : unsigned char {
  None = 0,
  Remote,
};

struct RustBridge {
  HANDLE pipe = INVALID_HANDLE_VALUE;
  EngineBackend backend = EngineBackend::None;
  bool initialized = false;
  uint32_t pendingModeFlags = 0;
  std::filesystem::path loadedLexiconDir;
  std::wstring buildId;
  std::wstring cacheSignature;
};

std::mutex g_mutex;
RustBridge g_bridge;
std::atomic<uint32_t> g_pendingModeFlagsMirror{0};
std::atomic<SrfEngineState> g_engineState = SrfEngineState::Idle;
std::atomic<bool> g_warmupInFlight = false;
std::atomic<bool> g_retryOnFailureEnabled = false;
std::atomic<bool> g_retryLoopInFlight = false;
std::atomic<unsigned long long> g_retryLoopGeneration = 0;
std::atomic<ULONGLONG> g_lastEngineUseTime{0};
std::atomic<ULONGLONG> g_lastInteractiveLookupTick{0};
std::atomic<ULONGLONG> g_lastUserTriggeredWarmupTick{0};
std::atomic<unsigned> g_consecutiveLookupTimeouts{0};
std::atomic<ULONGLONG> g_lastLookupTimeoutRestartTick{0};
std::atomic<unsigned long long> g_latestLookupRequestId{0};
std::atomic<unsigned long long> g_latestLookupCancelRequestId{0};
std::atomic<bool> g_lookupCancelWorkerRunning{false};
std::atomic<bool> g_idleWatcherRunning{false};
std::atomic<bool> g_trayWatchdogRunning{false};
std::wstring g_lastEngineFailure;

struct LocalLookupCacheKey {
  std::wstring reading;
  uint32_t modeFlags = 0;
  std::wstring cacheSignature;
};

struct LocalLookupCacheEntry {
  LocalLookupCacheKey key;
  std::vector<std::wstring> candidates;
  std::vector<std::wstring> meta;
  ULONGLONG tick = 0;
};

std::mutex g_localLookupCacheMutex;
std::vector<LocalLookupCacheEntry> g_localLookupCache;
std::wstring g_localLookupCacheSignature;
constexpr size_t kLocalLookupCacheCapacity = 512;
constexpr ULONGLONG kLocalLookupCacheTtlMs = 60 * 1000;
constexpr ULONGLONG kLocalLookupCacheStaleTtlMs = 5 * 60 * 1000;
std::array<LocalLookupCacheEntry, 26> g_singleLetterLookupCache;
std::array<bool, 26> g_singleLetterLookupCacheValid = {};
std::atomic<ULONGLONG> g_lastRuntimePayloadCacheCheckTick{0};
std::atomic<bool> g_runtimePayloadStaleForCache{false};
constexpr ULONGLONG kSingleLetterLookupCacheTtlMs = 5 * 60 * 1000;
constexpr ULONGLONG kSingleLetterLookupCacheStaleTtlMs = 10 * 60 * 1000;
constexpr DWORD kSingleLetterReadyWaitMs = 8;
constexpr DWORD kLookupBusyWaitMs = 16;
constexpr ULONGLONG kPrewarmQuietAfterInteractiveLookupMs = 1500;
std::atomic<bool> g_singleLetterPrewarmInFlight{false};
constexpr const wchar_t* kHotLookupPrewarmReadings[] = {
    L"ni",   L"wo",   L"shi",  L"de",   L"le",   L"bu",   L"zai",  L"you",
    L"hao",  L"ma",   L"yao",  L"dao",  L"shuo", L"kan",  L"lai",  L"qu",
    L"hui",  L"neng", L"mei",  L"yi",   L"ge",   L"he",   L"ta",   L"men",
    L"zhong", L"guo", L"ren",  L"shang", L"xia", L"jin",  L"tian", L"ming"};

// Prewarm every key-path prefix through the fourth letter, not just the final
// common reading.  For example, "zhong" contributes zh/zho/zhon/zhong.  This
// lets the first visible candidate page for common input stay in the TIP-side
// cache instead of paying a named-pipe round trip on each key stroke.
const std::vector<std::wstring>& HotLookupPrewarmReadings() {
  static const std::vector<std::wstring> readings = [] {
    std::vector<std::wstring> result;
    auto appendUnique = [&](std::wstring reading) {
      if (reading.size() < 2 ||
          std::find(result.begin(), result.end(), reading) != result.end()) {
        return;
      }
      result.push_back(std::move(reading));
    };
    for (const wchar_t* raw : kHotLookupPrewarmReadings) {
      if (!raw || !*raw) continue;
      const std::wstring reading(raw);
      const size_t prefixLimit = (std::min)(reading.size(), static_cast<size_t>(4));
      for (size_t length = 2; length <= prefixLimit; ++length) {
        appendUnique(reading.substr(0, length));
      }
      if (reading.size() > prefixLimit) appendUnique(reading);
    }
    return result;
  }();
  return readings;
}

enum class PendingLearnKind : unsigned char {
  Commit,
  Correction,
  SelectionFeedback,
};

struct PendingLearnRequest {
  PendingLearnKind kind = PendingLearnKind::Commit;
  std::wstring reading;
  std::wstring committedText;
  std::wstring correctedReading;
  std::vector<std::wstring> skippedCandidates;
  unsigned long selectedIndex = 0;
  unsigned long page = 0;
  unsigned long flags = 0;
  unsigned int repeatCount = 1;
  HWND completionWindow = nullptr;
  UINT completionMessage = 0;
  unsigned long long completionId = 0;
};

std::mutex g_learnQueueMutex;
std::deque<PendingLearnRequest> g_learnQueue;
std::atomic<bool> g_learnWorkerRunning{false};
std::atomic<unsigned long long> g_nextLearnCompletionId{1};
constexpr size_t kLearnQueueCapacity = 128;
constexpr unsigned int kLearnQueueMaxRepeatCount = 16;

template <typename Fn>
bool StartDetachedBackgroundWorker(Fn&& fn) {
  SrfTip_BackgroundWorkerAddRef();
  try {
    std::thread([func = std::forward<Fn>(fn)]() mutable {
      try {
        func();
      } catch (...) {
      }
      SrfTip_BackgroundWorkerRelease();
    }).detach();
    return true;
  } catch (...) {
    SrfTip_BackgroundWorkerRelease();
    return false;
  }
}

std::vector<BYTE>& RemoteLookupPayloadScratch() {
  thread_local std::vector<BYTE> scratch;
  return scratch;
}

std::vector<BYTE>& RemoteLookupResponseScratch() {
  thread_local std::vector<BYTE> scratch;
  return scratch;
}

std::wstring WideFromUtf8(const char* value) {
  if (!value || !*value) return {};
  const int required = MultiByteToWideChar(CP_UTF8, 0, value, -1, nullptr, 0);
  if (required <= 1) return {};
  std::wstring out(static_cast<size_t>(required), L'\0');
  MultiByteToWideChar(CP_UTF8, 0, value, -1, out.data(), required);
  if (!out.empty() && out.back() == L'\0') out.pop_back();
  return out;
}

std::wstring ExpectedEngineBuildIdPrefix() {
  std::wstring id = L"kaixin-";
  id += WideFromUtf8(SRF_APP_VERSION);
  id += L"+git.";
  return id;
}

std::wstring ExpectedEngineBuildId() {
  std::wstring id = ExpectedEngineBuildIdPrefix();
  id += WideFromUtf8(SRF_GIT_COMMIT);
#if SRF_GIT_DIRTY
  id += L".dirty";
#endif
  return id;
}

bool EngineBuildIdHasExpectedVersion(const std::wstring& buildId) {
  const std::wstring prefix = ExpectedEngineBuildIdPrefix();
  return buildId.size() > prefix.size() &&
         _wcsnicmp(buildId.c_str(), prefix.c_str(), prefix.size()) == 0;
}

bool LocalLookupCacheKeyEquals(const LocalLookupCacheKey& a, const LocalLookupCacheKey& b) {
  return a.modeFlags == b.modeFlags && a.reading == b.reading &&
         a.cacheSignature == b.cacheSignature;
}

std::wstring LowerAsciiWide(std::wstring text) {
  for (wchar_t& ch : text) {
    if (ch >= L'A' && ch <= L'Z') ch = static_cast<wchar_t>(ch - L'A' + L'a');
  }
  return text;
}

int SingleLetterIndex(const std::wstring& reading) {
  if (reading.size() != 1) return -1;
  wchar_t ch = reading[0];
  if (ch >= L'A' && ch <= L'Z') ch = static_cast<wchar_t>(ch - L'A' + L'a');
  if (ch < L'a' || ch > L'z') return -1;
  return static_cast<int>(ch - L'a');
}

bool IsDynamicDirectLookupReading(const std::wstring& lower) {
  return lower == L"rq" || lower == L"jr" || lower == L"date" || lower == L"sj" ||
         lower == L"time" || lower == L"xq" || lower == L"week" || lower == L"zhou";
}

bool IsLocalLookupCacheableReading(const std::wstring& reading) {
  if (reading.empty() || reading.size() > 16) return false;
  std::wstring lower = LowerAsciiWide(reading);
  if (lower.rfind(L"vv", 0) == 0 || IsDynamicDirectLookupReading(lower)) return false;
  for (wchar_t ch : lower) {
    if (!((ch >= L'a' && ch <= L'z') || ch == L'\'')) return false;
  }
  return true;
}

bool IsExpiredLocalLookupCacheAllowed(const std::wstring& lower) {
  return !lower.empty();
}

bool IsHotLocalLookupCacheReading(const std::wstring& lower) {
  if (SingleLetterIndex(lower) >= 0) return true;
  for (const auto& hot : HotLookupPrewarmReadings()) {
    if (lower == hot) return true;
  }
  return false;
}

bool LookupResultHasPartialMeta(const std::vector<std::wstring>* metaScores) {
  if (!metaScores) return false;
  for (const std::wstring& meta : *metaScores) {
    if (meta.find(L"partial=1") != std::wstring::npos) return true;
  }
  return false;
}

bool IsLoadedRuntimePayloadStaleForCache();

void ClearLocalLookupCache() {
  std::lock_guard<std::mutex> guard(g_localLookupCacheMutex);
  g_localLookupCache.clear();
  g_singleLetterLookupCacheValid.fill(false);
}

void ClearLocalLookupCacheInternal() {
  g_localLookupCache.clear();
  g_singleLetterLookupCacheValid.fill(false);
}

void PruneLocalLookupCachePreservingHotInternal() {
  g_localLookupCache.erase(
      std::remove_if(g_localLookupCache.begin(), g_localLookupCache.end(),
                     [](const LocalLookupCacheEntry& entry) {
                       return !IsHotLocalLookupCacheReading(entry.key.reading);
                     }),
      g_localLookupCache.end());
}

void PruneLocalLookupCachePreservingHot() {
  std::lock_guard<std::mutex> guard(g_localLookupCacheMutex);
  PruneLocalLookupCachePreservingHotInternal();
}

void InvalidateLocalLookupCacheReading(const std::wstring& reading) {
  const std::wstring lower = LowerAsciiWide(reading);
  if (lower.empty()) return;
  std::lock_guard<std::mutex> guard(g_localLookupCacheMutex);
  const int singleIndex = SingleLetterIndex(lower);
  if (singleIndex >= 0) {
    g_singleLetterLookupCacheValid[static_cast<size_t>(singleIndex)] = false;
  }
  g_localLookupCache.erase(
      std::remove_if(g_localLookupCache.begin(), g_localLookupCache.end(),
                     [&](const LocalLookupCacheEntry& entry) {
                       return entry.key.reading == lower;
                     }),
      g_localLookupCache.end());
}

void SetLocalLookupCacheSignature(const std::wstring& signature) {
  std::lock_guard<std::mutex> guard(g_localLookupCacheMutex);
  if (g_localLookupCacheSignature == signature) return;
  g_localLookupCacheSignature = signature;
  ClearLocalLookupCacheInternal();
}

std::wstring CurrentLocalLookupCacheSignatureLocked() {
  return g_localLookupCacheSignature;
}

bool LocalLookupCacheSignatureMatchesCurrentTip(const std::wstring& signature) {
  if (signature.empty()) return false;
  const std::wstring expected = L"build=" + ExpectedEngineBuildIdPrefix();
  return signature.find(expected) != std::wstring::npos;
}

bool TryGetLocalLookupCacheInternal(const std::wstring& reading, uint32_t modeFlags,
                                    std::vector<std::wstring>& candidates,
                                    std::vector<std::wstring>* metaScores,
                                    bool allowExpired) {
  if (!IsLocalLookupCacheableReading(reading)) return false;
  if (IsLoadedRuntimePayloadStaleForCache()) return false;
  std::lock_guard<std::mutex> guard(g_localLookupCacheMutex);
  const std::wstring signature = CurrentLocalLookupCacheSignatureLocked();
  if (!LocalLookupCacheSignatureMatchesCurrentTip(signature)) {
    g_localLookupCacheSignature.clear();
    ClearLocalLookupCacheInternal();
    return false;
  }
  const LocalLookupCacheKey key{LowerAsciiWide(reading), modeFlags, signature};
  const ULONGLONG now = GetTickCount64();
  const int singleIndex = SingleLetterIndex(key.reading);
  if (singleIndex >= 0 && g_singleLetterLookupCacheValid[static_cast<size_t>(singleIndex)]) {
    LocalLookupCacheEntry& entry = g_singleLetterLookupCache[static_cast<size_t>(singleIndex)];
    const ULONGLONG age = now >= entry.tick ? now - entry.tick : 0;
    const bool expired = age > kSingleLetterLookupCacheTtlMs;
    const bool staleAllowed =
        expired && allowExpired && IsExpiredLocalLookupCacheAllowed(key.reading) &&
        age <= kSingleLetterLookupCacheStaleTtlMs;
    if ((!expired || staleAllowed) && LocalLookupCacheKeyEquals(entry.key, key) &&
        !entry.candidates.empty()) {
      if (!expired) entry.tick = now;
      candidates = entry.candidates;
      if (metaScores) *metaScores = entry.meta;
      return true;
    }
    if (expired && !staleAllowed) {
      g_singleLetterLookupCacheValid[static_cast<size_t>(singleIndex)] = false;
    }
  }
  for (size_t i = 0; i < g_localLookupCache.size();) {
    const ULONGLONG age =
        now >= g_localLookupCache[i].tick ? now - g_localLookupCache[i].tick : 0;
    const bool expired = age > kLocalLookupCacheTtlMs;
    const bool staleAllowed =
        expired && allowExpired && IsExpiredLocalLookupCacheAllowed(key.reading) &&
        age <= kLocalLookupCacheStaleTtlMs;
    if (expired && !staleAllowed) {
      g_localLookupCache.erase(g_localLookupCache.begin() + static_cast<std::ptrdiff_t>(i));
      continue;
    }
    if (LocalLookupCacheKeyEquals(g_localLookupCache[i].key, key)) {
      LocalLookupCacheEntry entry = std::move(g_localLookupCache[i]);
      g_localLookupCache.erase(g_localLookupCache.begin() + static_cast<std::ptrdiff_t>(i));
      candidates = entry.candidates;
      if (metaScores) *metaScores = entry.meta;
      if (expired) {
        g_localLookupCache.insert(g_localLookupCache.begin() + static_cast<std::ptrdiff_t>(i),
                                  std::move(entry));
        return !candidates.empty();
      }
      entry.tick = now;
      g_localLookupCache.push_back(std::move(entry));
      return !candidates.empty();
    }
    ++i;
  }
  return false;
}

bool TryGetLocalLookupCache(const std::wstring& reading, uint32_t modeFlags,
                            std::vector<std::wstring>& candidates,
                            std::vector<std::wstring>* metaScores) {
  return TryGetLocalLookupCacheInternal(reading, modeFlags, candidates, metaScores, false);
}

bool TryGetStaleLocalLookupCache(const std::wstring& reading, uint32_t modeFlags,
                                 std::vector<std::wstring>& candidates,
                                 std::vector<std::wstring>* metaScores) {
  return TryGetLocalLookupCacheInternal(reading, modeFlags, candidates, metaScores, true);
}

void PutLocalLookupCache(const std::wstring& reading, uint32_t modeFlags,
                          const std::vector<std::wstring>& candidates,
                          const std::vector<std::wstring>* metaScores) {
  if (candidates.empty() || !IsLocalLookupCacheableReading(reading)) return;
  if (IsLoadedRuntimePayloadStaleForCache()) return;
  if (LookupResultHasPartialMeta(metaScores)) return;
  LocalLookupCacheEntry entry;
  entry.key.reading = LowerAsciiWide(reading);
  entry.key.modeFlags = modeFlags;

  std::lock_guard<std::mutex> guard(g_localLookupCacheMutex);
  if (!LocalLookupCacheSignatureMatchesCurrentTip(g_localLookupCacheSignature)) {
    g_localLookupCacheSignature.clear();
    ClearLocalLookupCacheInternal();
    return;
  }
  entry.key.cacheSignature = g_localLookupCacheSignature;
  entry.candidates = candidates;
  if (metaScores) entry.meta = *metaScores;
  entry.tick = GetTickCount64();
  const int singleIndex = SingleLetterIndex(entry.key.reading);
  if (singleIndex >= 0) {
    g_singleLetterLookupCache[static_cast<size_t>(singleIndex)] = entry;
    g_singleLetterLookupCacheValid[static_cast<size_t>(singleIndex)] = true;
  }
  g_localLookupCache.erase(
      std::remove_if(g_localLookupCache.begin(), g_localLookupCache.end(),
                     [&](const LocalLookupCacheEntry& existing) {
                       return LocalLookupCacheKeyEquals(existing.key, entry.key);
                     }),
      g_localLookupCache.end());
  g_localLookupCache.push_back(std::move(entry));
  while (g_localLookupCache.size() > kLocalLookupCacheCapacity) {
    g_localLookupCache.erase(g_localLookupCache.begin());
  }
}

std::wstring ToLower(std::wstring value) {
  std::transform(value.begin(), value.end(), value.begin(),
                 [](wchar_t ch) { return static_cast<wchar_t>(towlower(ch)); });
  return value;
}

std::filesystem::path ModuleDir() {
  HMODULE module = nullptr;
  if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
                              GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                          reinterpret_cast<LPCWSTR>(&SrfTip_InitializeEngine), &module) ||
      !module) {
    return {};
  }
  wchar_t path[MAX_PATH] = {};
  if (!GetModuleFileNameW(module, path, MAX_PATH)) return {};
  return std::filesystem::path(path).parent_path();
}

std::filesystem::path FindHelper(const std::filesystem::path& moduleDir, const wchar_t* exeName) {
  std::error_code ec;
  std::filesystem::path current = moduleDir;
  // moduleDir sits under runtime\pkg-...\x64|x86, so the install root is one level
  // farther up than the old search depth reached.
  for (int depth = 0; depth < 4 && !current.empty(); ++depth) {
    const auto candidate = current / exeName;
    if (!candidate.empty() && std::filesystem::is_regular_file(candidate, ec)) return candidate;
    ec.clear();
    current = current.parent_path();
  }
  return {};
}

std::filesystem::path NormalizePath(const std::filesystem::path& path);
bool ResolveTrustedInstallLayout(const std::filesystem::path& moduleDir,
                                 std::filesystem::path* installRoot,
                                 std::filesystem::path* runtimeRoot);

uint64_t StablePathHash(const std::filesystem::path& path) {
  const std::wstring text = ToLower(NormalizePath(path).wstring());
  uint64_t hash = 1469598103934665603ull;
  for (wchar_t ch : text) {
    hash ^= static_cast<uint16_t>(ch);
    hash *= 1099511628211ull;
  }
  return hash;
}

std::wstring Hex64(uint64_t value) {
  static constexpr wchar_t kHex[] = L"0123456789abcdef";
  std::wstring out(16, L'0');
  for (int i = 15; i >= 0; --i) {
    out[static_cast<size_t>(i)] = kHex[value & 0x0f];
    value >>= 4;
  }
  return out;
}

std::wstring EngineInstanceSuffixForModuleDir(const std::filesystem::path& moduleDir) {
  std::filesystem::path installRoot;
  if (!ResolveTrustedInstallLayout(moduleDir, &installRoot, nullptr) || installRoot.empty()) {
    return {};
  }
  return Hex64(StablePathHash(installRoot));
}

std::wstring EnginePipeNameForModuleDir(const std::filesystem::path& moduleDir) {
  const std::wstring suffix = EngineInstanceSuffixForModuleDir(moduleDir);
  if (suffix.empty()) return kDefaultEnginePipeName;
  return std::wstring(kEnginePipePrefix) + suffix;
}

std::wstring EngineMutexNameForModuleDir(const std::filesystem::path& moduleDir) {
  const std::wstring suffix = EngineInstanceSuffixForModuleDir(moduleDir);
  if (suffix.empty()) return kDefaultEngineMutexName;
  return std::wstring(kEngineMutexPrefix) + suffix;
}

bool AssignEngineHelperJob(HANDLE process) {
  static HANDLE job = nullptr;
  if (!process) return false;
  if (!job) {
    job = CreateJobObjectW(nullptr, L"Local\\KaixinInput_Engine_Job");
    if (!job) return false;

    JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits = {};
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    limits.ProcessMemoryLimit = kEngineHelperMemoryLimitBytes;
    if (!SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits,
                                 sizeof(limits))) {
      CloseHandle(job);
      job = nullptr;
      return false;
    }
  }
  return AssignProcessToJobObject(job, process) != FALSE;
}

bool InstallMaintenanceActive() {
  DWORD value = 0;
  DWORD cb = sizeof(value);
  if (RegGetValueW(HKEY_CURRENT_USER, kStateRegPath, kStateInstallMaintenanceValue,
                   RRF_RT_REG_DWORD, nullptr, &value, &cb) != ERROR_SUCCESS || value == 0) {
    return false;
  }

  // A killed installer must not disable the input engine forever. New
  // installers publish a boot-relative tick together with the flag. A flag
  // from an older installer (or from a previous boot) has no valid lease and
  // is therefore treated as stale.
  DWORD maintenanceTick = 0;
  cb = sizeof(maintenanceTick);
  if (RegGetValueW(HKEY_CURRENT_USER, kStateRegPath, kStateInstallMaintenanceTickValue,
                   RRF_RT_REG_DWORD, nullptr, &maintenanceTick, &cb) != ERROR_SUCCESS) {
    return false;
  }
  return GetTickCount() - maintenanceTick <= kInstallMaintenanceMaxAgeMs;
}

bool EnsureEngineHelperRunning(const std::filesystem::path& moduleDir) {
  if (InstallMaintenanceActive()) return false;

  const std::wstring mutexName = EngineMutexNameForModuleDir(moduleDir);
  HANDLE mutex = OpenMutexW(SYNCHRONIZE, FALSE, mutexName.c_str());
  if (mutex) {
    CloseHandle(mutex);
    return false;
  }

  auto helperExe = FindHelper(moduleDir, L"srf_ime_engine.exe");
  if (helperExe.empty()) return false;

  const std::wstring pipeName = EnginePipeNameForModuleDir(moduleDir);
  std::wstring commandLine = L"\"" + helperExe.wstring() + L"\" --pipe-name \"" + pipeName +
                             L"\" --mutex-name \"" + mutexName + L"\"";
  std::wstring workDir = helperExe.parent_path().wstring();

  STARTUPINFOW startup = {};
  startup.cb = sizeof(startup);
  PROCESS_INFORMATION processInfo = {};
  bool started = false;
  if (CreateProcessW(helperExe.c_str(), commandLine.data(), nullptr, nullptr, FALSE,
                     CREATE_DEFAULT_ERROR_MODE | CREATE_SUSPENDED, nullptr,
                     workDir.empty() ? nullptr : workDir.c_str(), &startup, &processInfo)) {
    // Browser renderers, packaged apps, and IDE hosts commonly run inside
    // their own jobs. Our memory-limit job is optional; failure to join it
    // must not make the entire input method unavailable.
    (void)AssignEngineHelperJob(processInfo.hProcess);
    if (ResumeThread(processInfo.hThread) == static_cast<DWORD>(-1)) {
      TerminateProcess(processInfo.hProcess, 1);
    } else {
      started = true;
    }
    CloseHandle(processInfo.hThread);
    CloseHandle(processInfo.hProcess);
  }
  return started;
}

bool WaitForEnginePipeReady(const std::filesystem::path& moduleDir, DWORD timeoutMs) {
  const std::wstring pipeName = EnginePipeNameForModuleDir(moduleDir);
  const ULONGLONG deadline = GetTickCount64() + timeoutMs;
  do {
    if (WaitNamedPipeW(pipeName.c_str(), 1)) return true;
    const DWORD error = GetLastError();
    if (error != ERROR_FILE_NOT_FOUND && error != ERROR_PIPE_BUSY && error != ERROR_SEM_TIMEOUT) {
      return false;
    }
    Sleep(20);
  } while (GetTickCount64() < deadline);
  return false;
}

std::filesystem::path NormalizePath(const std::filesystem::path& path) {
  if (path.empty()) return {};
  std::error_code ec;
  auto absolute = std::filesystem::absolute(path, ec);
  auto normalized = (ec ? path : absolute).lexically_normal();
  auto canonical = std::filesystem::weakly_canonical(normalized, ec);
  return ec ? normalized : canonical;
}

bool PathEquals(const std::filesystem::path& lhs, const std::filesystem::path& rhs) {
  return ToLower(NormalizePath(lhs).wstring()) == ToLower(NormalizePath(rhs).wstring());
}

bool PathStartsWith(const std::filesystem::path& path, const std::filesystem::path& root) {
  const auto normalizedPath = NormalizePath(path);
  const auto normalizedRoot = NormalizePath(root);
  if (normalizedPath.empty() || normalizedRoot.empty()) return false;

  auto pathIt = normalizedPath.begin();
  auto rootIt = normalizedRoot.begin();
  for (; rootIt != normalizedRoot.end(); ++rootIt, ++pathIt) {
    if (pathIt == normalizedPath.end()) return false;
    if (ToLower(pathIt->wstring()) != ToLower(rootIt->wstring())) return false;
  }
  return true;
}

std::filesystem::path GetEnvPath(const wchar_t* name) {
  const DWORD len = GetEnvironmentVariableW(name, nullptr, 0);
  if (len == 0) return {};
  std::wstring value(len, L'\0');
  if (GetEnvironmentVariableW(name, value.data(), len) == 0) return {};
  if (!value.empty() && value.back() == L'\0') value.pop_back();
  return value.empty() ? std::filesystem::path() : std::filesystem::path(value);
}

std::filesystem::path UserInstallRoot(const wchar_t* appName) {
  const auto localAppData = GetEnvPath(L"LOCALAPPDATA");
  return localAppData.empty() ? std::filesystem::path()
                              : localAppData / L"Programs" / appName;
}

std::vector<std::filesystem::path> MachineInstallRoots(const wchar_t* appName) {
  std::vector<std::filesystem::path> roots;
  const wchar_t* envNames[] = {L"ProgramW6432", L"ProgramFiles", L"ProgramFiles(x86)"};
  for (const wchar_t* envName : envNames) {
    const auto base = GetEnvPath(envName);
    if (base.empty()) continue;
    const auto root = base / appName;
    const bool exists = std::any_of(roots.begin(), roots.end(), [&](const auto& existing) {
      return PathEquals(existing, root);
    });
    if (!exists) roots.push_back(root);
  }
  return roots;
}

std::filesystem::path UserDataRoot(const wchar_t* appName = kAppPathName) {
  const auto localAppData = GetEnvPath(L"LOCALAPPDATA");
  return localAppData.empty() ? std::filesystem::path() : localAppData / appName;
}

bool IsTrustedInstallRoot(const std::filesystem::path& path) {
  const auto userRoot = UserInstallRoot(kAppPathName);
  if (!userRoot.empty() && PathEquals(path, userRoot)) {
    return true;
  }
  for (const auto& machineRoot : MachineInstallRoots(kAppPathName)) {
    if (!machineRoot.empty() && PathEquals(path, machineRoot)) return true;
  }
  return false;
}

bool TryResolveTrustedInstallLayoutForRoot(const std::filesystem::path& moduleDir,
                                           const std::filesystem::path& trustedRoot,
                                           std::filesystem::path* installRoot,
                                           std::filesystem::path* runtimeRoot) {
  if (trustedRoot.empty()) return false;
  if (PathEquals(moduleDir, trustedRoot)) {
    if (installRoot) *installRoot = trustedRoot;
    if (runtimeRoot) *runtimeRoot = trustedRoot;
    return true;
  }

  const auto runtimeParent = trustedRoot / L"runtime";
  if (PathEquals(moduleDir.parent_path(), runtimeParent)) {
    // The installer can move old payload directories while host processes still
    // have this DLL mapped. Trust the loaded module path even if the file no
    // longer exists at its original location.
    if (installRoot) *installRoot = trustedRoot;
    if (runtimeRoot) *runtimeRoot = moduleDir;
    return true;
  }

  const std::wstring archDirName = moduleDir.filename().wstring();
  const bool isArchRuntimeDir = _wcsicmp(archDirName.c_str(), L"x64") == 0 ||
                                _wcsicmp(archDirName.c_str(), L"x86") == 0;
  if (!isArchRuntimeDir) return false;
  const auto payloadDir = moduleDir.parent_path();
  if (!PathEquals(payloadDir.parent_path(), runtimeParent)) return false;

  if (installRoot) *installRoot = trustedRoot;
  if (runtimeRoot) *runtimeRoot = moduleDir;
  return true;
}

bool ResolveTrustedInstallLayout(const std::filesystem::path& moduleDir,
                                  std::filesystem::path* installRoot,
                                  std::filesystem::path* runtimeRoot) {
  if (TryResolveTrustedInstallLayoutForRoot(moduleDir, UserInstallRoot(kAppPathName), installRoot,
                                            runtimeRoot)) {
    return true;
  }
  for (const auto& machineRoot : MachineInstallRoots(kAppPathName)) {
    if (TryResolveTrustedInstallLayoutForRoot(moduleDir, machineRoot, installRoot, runtimeRoot)) {
      return true;
    }
  }
  return false;
}

std::wstring TrimAsciiWhitespace(std::wstring value) {
  while (!value.empty() && (value.back() == L' ' || value.back() == L'\t' ||
                            value.back() == L'\r' || value.back() == L'\n')) {
    value.pop_back();
  }
  size_t start = 0;
  while (start < value.size() && (value[start] == L' ' || value[start] == L'\t' ||
                                  value[start] == L'\r' || value[start] == L'\n')) {
    ++start;
  }
  return start == 0 ? value : value.substr(start);
}

std::filesystem::path CurrentRuntimePayloadRoot(const std::filesystem::path& installRoot) {
  if (installRoot.empty()) return {};
  const auto markerPath = installRoot / L"current_runtime_payload.txt";
  std::wifstream file(markerPath);
  if (!file) return {};

  std::wstring text;
  std::getline(file, text);
  text = TrimAsciiWhitespace(text);
  if (text.empty()) return {};

  std::filesystem::path markerValue(text);
  if (markerValue.is_absolute()) return {};

  const auto payloadRoot = NormalizePath(installRoot / markerValue);
  if (payloadRoot.empty() || !PathStartsWith(payloadRoot, installRoot)) return {};
  return payloadRoot;
}

bool IsArchRuntimeDirName(const std::filesystem::path& path) {
  const std::wstring name = path.filename().wstring();
  return _wcsicmp(name.c_str(), L"x64") == 0 || _wcsicmp(name.c_str(), L"x86") == 0;
}

std::filesystem::path RuntimePayloadRootForModule(const std::filesystem::path& installRoot,
                                                  const std::filesystem::path& runtimeRoot) {
  if (installRoot.empty() || runtimeRoot.empty()) return {};
  const auto runtimeParent = installRoot / L"runtime";
  if (PathEquals(runtimeRoot.parent_path(), runtimeParent)) {
    return NormalizePath(runtimeRoot);
  }
  if (IsArchRuntimeDirName(runtimeRoot) && PathEquals(runtimeRoot.parent_path().parent_path(), runtimeParent)) {
    return NormalizePath(runtimeRoot.parent_path());
  }
  return {};
}

bool IsStaleRuntimePayload(const std::filesystem::path& installRoot,
                           const std::filesystem::path& runtimeRoot) {
  const auto currentPayloadRoot = CurrentRuntimePayloadRoot(installRoot);
  if (currentPayloadRoot.empty()) return false;
  const auto modulePayloadRoot = RuntimePayloadRootForModule(installRoot, runtimeRoot);
  if (modulePayloadRoot.empty()) return false;
  return !PathEquals(modulePayloadRoot, currentPayloadRoot);
}

bool IsLoadedRuntimePayloadStaleForCache() {
  const ULONGLONG now = GetTickCount64();
  const ULONGLONG last = g_lastRuntimePayloadCacheCheckTick.load(std::memory_order_acquire);
  if (last != 0 && now >= last && now - last < 1000) {
    return g_runtimePayloadStaleForCache.load(std::memory_order_acquire);
  }
  g_lastRuntimePayloadCacheCheckTick.store(now, std::memory_order_release);

  bool stale = false;
  const auto moduleDir = ModuleDir();
  if (!moduleDir.empty()) {
    std::filesystem::path installRoot;
    std::filesystem::path runtimeRoot;
    if (ResolveTrustedInstallLayout(moduleDir, &installRoot, &runtimeRoot)) {
      stale = IsStaleRuntimePayload(installRoot, runtimeRoot);
    }
  }
  g_runtimePayloadStaleForCache.store(stale, std::memory_order_release);
  if (stale) ClearLocalLookupCache();
  return stale;
}

bool IsTrustedUserDataPath(const std::filesystem::path& path) {
  const auto userRoot = UserDataRoot(kAppPathName);
  return !userRoot.empty() && PathStartsWith(path, userRoot);
}

void AppendEngineFailureLogDeduped(const std::wstring& detail);
void AppendEngineStateLogDeduped(const std::wstring& detail);

std::filesystem::path FindLexiconDirDirect(const std::filesystem::path& root) {
  if (root.empty() || !std::filesystem::is_directory(root)) return {};

  const auto lexiconDir = root / L"lexicon";
  if (std::filesystem::is_directory(lexiconDir)) return lexiconDir;

  const auto preferred = root / L"\u8bcd\u5e93-2026-03-26";
  if (std::filesystem::is_directory(preferred)) return preferred;

  std::error_code ec;
  for (const auto& entry : std::filesystem::directory_iterator(root, ec)) {
    if (ec) break;
    if (!entry.is_directory()) continue;
    const auto name = entry.path().filename().wstring();
    if (name.rfind(L"\u8bcd\u5e93-", 0) == 0) return entry.path();
  }

  return {};
}

std::wstring BytesToHex(const BYTE* data, size_t len) {
  static constexpr wchar_t kHex[] = L"0123456789abcdef";
  std::wstring out;
  out.reserve(len * 2);
  for (size_t i = 0; i < len; ++i) {
    out.push_back(kHex[(data[i] >> 4) & 0x0F]);
    out.push_back(kHex[data[i] & 0x0F]);
  }
  return out;
}

bool ReadExpectedSha256(const std::filesystem::path& path, std::wstring* out,
                        std::wstring* error) {
  std::ifstream file(path, std::ios::binary);
  if (!file) {
    if (error) *error = L"missing hash manifest: " + path.wstring();
    return false;
  }

  std::string text((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
  while (!text.empty() && isspace(static_cast<unsigned char>(text.back()))) text.pop_back();
  size_t start = 0;
  while (start < text.size() && isspace(static_cast<unsigned char>(text[start]))) ++start;
  text = text.substr(start);
  std::transform(text.begin(), text.end(), text.begin(),
                 [](unsigned char ch) { return static_cast<char>(tolower(ch)); });
  if (text.size() != 64 ||
      !std::all_of(text.begin(), text.end(), [](unsigned char ch) { return isxdigit(ch) != 0; })) {
    if (error) *error = L"invalid hash manifest contents: " + path.wstring();
    return false;
  }

  if (out) *out = std::wstring(text.begin(), text.end());
  return true;
}

bool ComputeSha256(const std::filesystem::path& path, std::wstring* out, std::wstring* error) {
  std::ifstream file(path, std::ios::binary);
  if (!file) {
    if (error) *error = L"cannot open file for hashing: " + path.wstring();
    return false;
  }

  HCRYPTPROV provider = 0;
  HCRYPTHASH hash = 0;
  if (!CryptAcquireContextW(&provider, nullptr, nullptr, PROV_RSA_AES, CRYPT_VERIFYCONTEXT)) {
    if (error) *error = L"CryptAcquireContextW failed";
    return false;
  }
  if (!CryptCreateHash(provider, CALG_SHA_256, 0, 0, &hash)) {
    if (error) *error = L"CryptCreateHash failed";
    CryptReleaseContext(provider, 0);
    return false;
  }

  std::array<char, 64 * 1024> buffer = {};
  while (file) {
    file.read(buffer.data(), static_cast<std::streamsize>(buffer.size()));
    const std::streamsize read = file.gcount();
    if (read > 0 &&
        !CryptHashData(hash, reinterpret_cast<const BYTE*>(buffer.data()), static_cast<DWORD>(read),
                       0)) {
      if (error) *error = L"CryptHashData failed";
      CryptDestroyHash(hash);
      CryptReleaseContext(provider, 0);
      return false;
    }
  }

  if (file.bad()) {
    if (error) *error = L"failed while reading file for hashing: " + path.wstring();
    CryptDestroyHash(hash);
    CryptReleaseContext(provider, 0);
    return false;
  }

  BYTE digest[32] = {};
  DWORD digestLen = sizeof(digest);
  if (!CryptGetHashParam(hash, HP_HASHVAL, digest, &digestLen, 0)) {
    if (error) *error = L"CryptGetHashParam failed";
    CryptDestroyHash(hash);
    CryptReleaseContext(provider, 0);
    return false;
  }

  CryptDestroyHash(hash);
  CryptReleaseContext(provider, 0);
  if (out) *out = BytesToHex(digest, digestLen);
  return true;
}

std::filesystem::path ResolveTrustedLexiconDir(const std::filesystem::path& moduleDir,
                                               std::wstring* error) {
  std::filesystem::path installRoot;
  std::filesystem::path runtimeRoot;
  if (!ResolveTrustedInstallLayout(moduleDir, &installRoot, &runtimeRoot)) {
    if (error) *error = L"untrusted install root for lexicon lookup";
    return {};
  }
  if (IsStaleRuntimePayload(installRoot, runtimeRoot)) {
    const auto currentPayloadRoot = CurrentRuntimePayloadRoot(installRoot);
    const auto modulePayloadRoot = RuntimePayloadRootForModule(installRoot, runtimeRoot);
    std::wstring detail = L"engine init: stale runtime payload tolerated";
    if (!modulePayloadRoot.empty()) {
      detail += L"; modulePayload=";
      detail += modulePayloadRoot.wstring();
    }
    if (!currentPayloadRoot.empty()) {
      detail += L"; currentPayload=";
      detail += currentPayloadRoot.wstring();
    }
    AppendEngineStateLogDeduped(detail);
  }

  const auto bundled = FindLexiconDirDirect(installRoot);
  if (!bundled.empty()) return bundled;

  const auto userLexiconRoot = UserDataRoot(kAppPathName) / L"lexicon";
  if (IsTrustedUserDataPath(userLexiconRoot)) {
    const auto userLexicon = FindLexiconDirDirect(userLexiconRoot);
    if (!userLexicon.empty()) return userLexicon;
  }

  if (error) *error = L"trusted lexicon directory not found";
  return {};
}

std::wstring TrimRight(std::wstring value) {
  while (!value.empty()) {
    const wchar_t ch = value.back();
    if (ch == L'\r' || ch == L'\n' || ch == L' ' || ch == L'\t') {
      value.pop_back();
      continue;
    }
    break;
  }
  return value;
}

std::wstring FormatSystemError(DWORD error) {
  if (error == 0) return L"unknown error";

  LPWSTR buffer = nullptr;
  const DWORD flags = FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM |
                      FORMAT_MESSAGE_IGNORE_INSERTS;
  const DWORD length =
      FormatMessageW(flags, nullptr, error, 0, reinterpret_cast<LPWSTR>(&buffer), 0, nullptr);
  if (length == 0 || !buffer) {
    return L"error " + std::to_wstring(error);
  }

  std::wstring message(buffer, length);
  LocalFree(buffer);
  return TrimRight(message);
}

void SetFailureDetailLocked(const std::wstring& detail) { g_lastEngineFailure = detail; }

void ClearFailureDetailLocked() { g_lastEngineFailure.clear(); }

std::string Utf8FromWide(const std::wstring& w) {
  if (w.empty()) return {};
  const int n = WideCharToMultiByte(CP_UTF8, 0, w.c_str(), static_cast<int>(w.size()), nullptr, 0,
                                    nullptr, nullptr);
  if (n <= 0) return {};
  std::string out(static_cast<size_t>(n), '\0');
  WideCharToMultiByte(CP_UTF8, 0, w.c_str(), static_cast<int>(w.size()), out.data(), n, nullptr,
                      nullptr);
  return out;
}

std::wstring LogValue(std::wstring value, size_t maxUnits = 240) {
  for (wchar_t& ch : value) {
    if (ch == L' ' || ch == L'\t' || ch == L'\r' || ch == L'\n' || ch == L',' ||
        ch == L';' || ch == L'=') {
      ch = L'_';
    }
  }
  if (value.size() > maxUnits) value.resize(maxUnits);
  return value.empty() ? L"(none)" : value;
}

void AppendEngineLogDeduped(const std::wstring& level, const std::wstring& event,
                            const std::wstring& detail) {
  static std::mutex s_logMutex;
  static std::wstring s_lastKey;
  static ULONGLONG s_lastTick = 0;
  std::lock_guard<std::mutex> guard(s_logMutex);
  const ULONGLONG now = GetTickCount64();
  const std::wstring key = level + L"\n" + event + L"\n" + detail;
  if (key == s_lastKey && now >= s_lastTick && now - s_lastTick < 60000ULL) return;
  s_lastKey = key;
  s_lastTick = now;

  const std::filesystem::path local = GetEnvPath(L"LOCALAPPDATA");
  if (local.empty()) return;
  std::error_code ec;
  const auto dir = local / kAppPathName / L"logs";
  std::filesystem::create_directories(dir, ec);
  if (ec) return;
  const auto logPath = dir / L"engine.log";

  SYSTEMTIME st {};
  GetLocalTime(&st);
  char timestamp[48];
  sprintf_s(timestamp, "%04u-%02u-%02u %02u:%02u:%02u ", static_cast<unsigned>(st.wYear),
            static_cast<unsigned>(st.wMonth), static_cast<unsigned>(st.wDay),
            static_cast<unsigned>(st.wHour), static_cast<unsigned>(st.wMinute),
            static_cast<unsigned>(st.wSecond));

  std::ofstream out(logPath, std::ios::app | std::ios::binary);
  if (!out) return;
  const std::wstring& line = detail.empty() ? L"(empty engine failure detail)" : detail;
  std::wstring structured = L"[" + level + L"] component=engine event=" + event + L" pid=";
  structured += std::to_wstring(GetCurrentProcessId());
  structured += L" tid=";
  structured += std::to_wstring(GetCurrentThreadId());
  structured += L" source=tsf_bridge detail=";
  structured += LogValue(line);
  out << timestamp << Utf8FromWide(structured) << '\n';
}

void AppendEngineFailureLogDeduped(const std::wstring& detail) {
  AppendEngineLogDeduped(L"error", L"tsf_bridge_failure", detail);
}

void AppendEngineStateLogDeduped(const std::wstring& detail) {
  AppendEngineLogDeduped(L"basic", L"tsf_bridge_state", detail);
}

std::wstring CurrentLocalTimestampText() {
  SYSTEMTIME st {};
  GetLocalTime(&st);
  wchar_t buffer[32] = {};
  swprintf_s(buffer, L"%04u-%02u-%02u %02u:%02u:%02u", static_cast<unsigned>(st.wYear),
             static_cast<unsigned>(st.wMonth), static_cast<unsigned>(st.wDay),
             static_cast<unsigned>(st.wHour), static_cast<unsigned>(st.wMinute),
             static_cast<unsigned>(st.wSecond));
  return buffer;
}

void WriteStateStringValue(const wchar_t* name, const std::wstring& value) {
  HKEY key = nullptr;
  if (RegCreateKeyExW(HKEY_CURRENT_USER, kStateRegPath, 0, nullptr, 0, KEY_SET_VALUE, nullptr,
                      &key, nullptr) != ERROR_SUCCESS) {
    return;
  }
  const DWORD bytes = static_cast<DWORD>((value.size() + 1) * sizeof(wchar_t));
  RegSetValueExW(key, name, 0, REG_SZ, reinterpret_cast<const BYTE*>(value.c_str()), bytes);
  RegCloseKey(key);
}

void WriteStateDwordValue(const wchar_t* name, DWORD value) {
  HKEY key = nullptr;
  if (RegCreateKeyExW(HKEY_CURRENT_USER, kStateRegPath, 0, nullptr, 0, KEY_SET_VALUE, nullptr,
                      &key, nullptr) != ERROR_SUCCESS) {
    return;
  }
  RegSetValueExW(key, name, 0, REG_DWORD, reinterpret_cast<const BYTE*>(&value), sizeof(value));
  RegCloseKey(key);
}

void PublishEngineState(SrfEngineState state) {
  const SrfEngineState previous = g_engineState.exchange(state, std::memory_order_acq_rel);
  if (previous == state) return;
  WriteStateDwordValue(kStateEngineStateValue, static_cast<DWORD>(state));
}

void WriteEngineRecoveryState(const std::wstring& reason) {
  WriteStateStringValue(kStateLastEngineRecoveryReasonValue,
                        reason.empty() ? L"(empty recovery reason)" : reason);
  WriteStateStringValue(kStateLastEngineRecoveryTimeValue, CurrentLocalTimestampText());
}

void AppendU16(std::vector<BYTE>& out, uint16_t value) {
  out.push_back(static_cast<BYTE>(value & 0xFF));
  out.push_back(static_cast<BYTE>((value >> 8) & 0xFF));
}

void AppendU32(std::vector<BYTE>& out, uint32_t value) {
  out.push_back(static_cast<BYTE>(value & 0xFF));
  out.push_back(static_cast<BYTE>((value >> 8) & 0xFF));
  out.push_back(static_cast<BYTE>((value >> 16) & 0xFF));
  out.push_back(static_cast<BYTE>((value >> 24) & 0xFF));
}

void AppendU64(std::vector<BYTE>& out, uint64_t value) {
  for (int i = 0; i < 8; ++i) {
    out.push_back(static_cast<BYTE>((value >> (i * 8)) & 0xFF));
  }
}

void AppendWideString(std::vector<BYTE>& out, const std::wstring& value) {
  AppendU32(out, static_cast<uint32_t>(value.size()));
  for (wchar_t ch : value) {
    AppendU16(out, static_cast<uint16_t>(ch));
  }
}

std::wstring WideFromAscii(const std::string& value) {
  std::wstring out;
  out.reserve(value.size());
  for (unsigned char ch : value) out.push_back(static_cast<wchar_t>(ch));
  return out;
}

std::string NarrowAsciiFromWide(const std::wstring& value) {
  std::string out;
  out.reserve(value.size());
  for (wchar_t ch : value) {
    if (ch <= 0 || ch > 0x7f) return {};
    out.push_back(static_cast<char>(ch));
  }
  return out;
}

std::string HexEncode(const std::vector<BYTE>& bytes) {
  static constexpr char kHex[] = "0123456789abcdef";
  std::string out;
  out.reserve(bytes.size() * 2);
  for (BYTE byte : bytes) {
    out.push_back(kHex[(byte >> 4) & 0x0f]);
    out.push_back(kHex[byte & 0x0f]);
  }
  return out;
}

void SecureZeroBytes(void* data, size_t bytes) {
  if (data && bytes > 0) SecureZeroMemory(data, bytes);
}

void SecureZeroVector(std::vector<BYTE>& bytes) {
  if (!bytes.empty()) SecureZeroBytes(bytes.data(), bytes.size());
}

void SecureZeroString(std::string& value) {
  if (!value.empty()) SecureZeroBytes(value.data(), value.size());
}

void SecureZeroWideString(std::wstring& value) {
  if (!value.empty()) SecureZeroBytes(value.data(), value.size() * sizeof(wchar_t));
}

std::wstring CurrentUserSidString() {
  HANDLE token = nullptr;
  if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) return {};

  DWORD needed = 0;
  GetTokenInformation(token, TokenUser, nullptr, 0, &needed);
  if (needed == 0) {
    CloseHandle(token);
    return {};
  }

  std::vector<BYTE> buffer(needed);
  if (!GetTokenInformation(token, TokenUser, buffer.data(), needed, &needed)) {
    CloseHandle(token);
    SecureZeroVector(buffer);
    return {};
  }
  CloseHandle(token);

  const TOKEN_USER* tokenUser = reinterpret_cast<const TOKEN_USER*>(buffer.data());
  LPWSTR sidText = nullptr;
  if (!ConvertSidToStringSidW(tokenUser->User.Sid, &sidText) || !sidText) {
    SecureZeroVector(buffer);
    return {};
  }
  std::wstring out = sidText;
  LocalFree(sidText);
  SecureZeroVector(buffer);
  return out;
}

bool RestrictFileToCurrentUser(const std::filesystem::path& path) {
  const std::wstring sid = CurrentUserSidString();
  if (sid.empty()) return false;

  std::wstring sddl = L"D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;";
  sddl += sid;
  sddl += L")";
  PSECURITY_DESCRIPTOR descriptor = nullptr;
  if (!ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.c_str(), SDDL_REVISION_1,
                                                            &descriptor, nullptr) ||
      !descriptor) {
    return false;
  }
  const BOOL ok =
      SetFileSecurityW(path.c_str(), DACL_SECURITY_INFORMATION, descriptor) != FALSE;
  LocalFree(descriptor);
  return ok == TRUE;
}

bool ReadAllBytes(const std::filesystem::path& path, std::vector<BYTE>* out) {
  if (!out) return false;
  out->clear();
  std::ifstream file(path, std::ios::binary);
  if (!file) return false;
  file.seekg(0, std::ios::end);
  const std::streamoff size = file.tellg();
  if (size < 0 || size > 1024 * 1024) return false;
  file.seekg(0, std::ios::beg);
  out->resize(static_cast<size_t>(size));
  if (!out->empty()) file.read(reinterpret_cast<char*>(out->data()), size);
  return file.good() || file.eof();
}

bool WriteAllBytes(const std::filesystem::path& path, const std::vector<BYTE>& bytes) {
  std::error_code ec;
  if (!path.parent_path().empty()) std::filesystem::create_directories(path.parent_path(), ec);
  std::ofstream file(path, std::ios::binary | std::ios::trunc);
  if (!file) return false;
  if (!bytes.empty()) file.write(reinterpret_cast<const char*>(bytes.data()), bytes.size());
  file.flush();
  if (!file.good()) return false;
  file.close();
  return file.good() && RestrictFileToCurrentUser(path);
}

bool ProtectData(const std::vector<BYTE>& plain, std::vector<BYTE>* protectedBytes) {
  if (!protectedBytes) return false;
  protectedBytes->clear();
  DATA_BLOB input = {};
  input.cbData = static_cast<DWORD>(plain.size());
  input.pbData = const_cast<BYTE*>(plain.data());
  DATA_BLOB output = {};
  if (!CryptProtectData(&input, nullptr, nullptr, nullptr, nullptr, CRYPTPROTECT_UI_FORBIDDEN,
                        &output)) {
    return false;
  }
  protectedBytes->assign(output.pbData, output.pbData + output.cbData);
  SecureZeroBytes(output.pbData, output.cbData);
  LocalFree(output.pbData);
  return true;
}

bool UnprotectData(const BYTE* data, size_t size, std::vector<BYTE>* plain) {
  if (!data || !plain || size == 0) return false;
  plain->clear();
  DATA_BLOB input = {};
  input.cbData = static_cast<DWORD>(size);
  input.pbData = const_cast<BYTE*>(data);
  DATA_BLOB output = {};
  if (!CryptUnprotectData(&input, nullptr, nullptr, nullptr, nullptr, CRYPTPROTECT_UI_FORBIDDEN,
                          &output)) {
    return false;
  }
  plain->assign(output.pbData, output.pbData + output.cbData);
  SecureZeroBytes(output.pbData, output.cbData);
  LocalFree(output.pbData);
  return true;
}

std::filesystem::path EngineCapabilityPath() {
  const auto root = UserDataRoot();
  return root.empty() ? std::filesystem::path() : root / kEngineCapabilityFileName;
}

std::wstring DecodeCapabilityTokenBytes(const std::vector<BYTE>& bytes) {
  if (bytes.empty()) return {};
  const BYTE* data = bytes.data();
  size_t size = bytes.size();
  constexpr size_t kMagicLen = sizeof(kEngineCapabilityMagic) - 1;
  if (size > kMagicLen && std::equal(data, data + kMagicLen, kEngineCapabilityMagic)) {
    std::vector<BYTE> plain;
    if (!UnprotectData(data + kMagicLen, size - kMagicLen, &plain)) return {};
    data = plain.data();
    size = plain.size();
    std::string token(reinterpret_cast<const char*>(data), size);
    std::wstring out = WideFromAscii(token);
    SecureZeroString(token);
    SecureZeroVector(plain);
    return out;
  }
  std::string token(reinterpret_cast<const char*>(data), size);
  std::wstring out = WideFromAscii(token);
  SecureZeroString(token);
  return out;
}

bool LooksLikeCapabilityToken(const std::wstring& token) {
  if (token.size() < 32 || token.size() > 128) return false;
  for (wchar_t ch : token) {
    const bool hex = (ch >= L'0' && ch <= L'9') || (ch >= L'a' && ch <= L'f') ||
                     (ch >= L'A' && ch <= L'F');
    if (!hex) return false;
  }
  return true;
}

std::wstring GenerateCapabilityToken() {
  std::vector<BYTE> randomBytes(32);
  HCRYPTPROV provider = 0;
  if (!CryptAcquireContextW(&provider, nullptr, nullptr, PROV_RSA_FULL, CRYPT_VERIFYCONTEXT)) {
    return {};
  }
  const BOOL ok = CryptGenRandom(provider, static_cast<DWORD>(randomBytes.size()), randomBytes.data());
  CryptReleaseContext(provider, 0);
  if (!ok) return {};
  return WideFromAscii(HexEncode(randomBytes));
}

std::wstring LoadOrCreateEngineCapabilityToken() {
  static std::mutex tokenMutex;
  std::lock_guard<std::mutex> guard(tokenMutex);

  const auto path = EngineCapabilityPath();
  if (path.empty()) return {};

  std::vector<BYTE> bytes;
  if (ReadAllBytes(path, &bytes)) {
    std::wstring token = DecodeCapabilityTokenBytes(bytes);
    SecureZeroVector(bytes);
    token = TrimAsciiWhitespace(token);
    if (LooksLikeCapabilityToken(token)) {
      return token;
    }
    SecureZeroWideString(token);
  }

  std::wstring token = GenerateCapabilityToken();
  if (!LooksLikeCapabilityToken(token)) return {};
  std::string tokenBytes = NarrowAsciiFromWide(token);
  if (tokenBytes.empty()) {
    SecureZeroWideString(token);
    return {};
  }

  std::vector<BYTE> plain(tokenBytes.begin(), tokenBytes.end());
  std::vector<BYTE> protectedBytes;
  if (!ProtectData(plain, &protectedBytes)) {
    SecureZeroVector(plain);
    SecureZeroString(tokenBytes);
    SecureZeroWideString(token);
    return {};
  }
  std::vector<BYTE> fileBytes;
  constexpr size_t kMagicLen = sizeof(kEngineCapabilityMagic) - 1;
  fileBytes.insert(fileBytes.end(), reinterpret_cast<const BYTE*>(kEngineCapabilityMagic),
                   reinterpret_cast<const BYTE*>(kEngineCapabilityMagic) + kMagicLen);
  fileBytes.insert(fileBytes.end(), protectedBytes.begin(), protectedBytes.end());
  const bool wrote = WriteAllBytes(path, fileBytes);
  SecureZeroVector(fileBytes);
  SecureZeroVector(protectedBytes);
  SecureZeroVector(plain);
  SecureZeroString(tokenBytes);
  if (!wrote) {
    SecureZeroWideString(token);
    return {};
  }

  return token;
}

bool AppendCapabilityToken(std::vector<BYTE>& out, std::wstring* error) {
  std::wstring token = LoadOrCreateEngineCapabilityToken();
  if (!LooksLikeCapabilityToken(token)) {
    SecureZeroWideString(token);
    if (error) *error = L"engine capability token unavailable";
    return false;
  }
  AppendWideString(out, token);
  SecureZeroWideString(token);
  return true;
}

const wchar_t* RemoteCommandLabel(EnginePipeCommand command) {
  switch (command) {
    case EnginePipeCommand::Init:
      return L"init";
    case EnginePipeCommand::Lookup:
      return L"lookup";
    case EnginePipeCommand::Learn:
      return L"learn";
    case EnginePipeCommand::SyllableBounds:
      return L"syllable-bounds";
    case EnginePipeCommand::RecordClipboard:
      return L"record-clipboard";
    case EnginePipeCommand::SetCandidatePin:
      return L"set-candidate-pin";
    case EnginePipeCommand::Health:
      return L"health";
    case EnginePipeCommand::ResolveClipboard:
      return L"resolve-clipboard";
    case EnginePipeCommand::Shutdown:
      return L"shutdown";
    case EnginePipeCommand::LearnCorrection:
      return L"learn-correction";
    case EnginePipeCommand::LearnSelectionFeedback:
      return L"learn-selection-feedback";
    case EnginePipeCommand::CandidateAction:
      return L"candidate-action";
    case EnginePipeCommand::ResetLearningContext:
      return L"reset-learning-context";
    case EnginePipeCommand::CancelLookup:
      return L"cancel-lookup";
  }
  return L"unknown";
}

std::wstring DecodeRemoteErrorPayload(const std::vector<BYTE>* payload) {
  if (!payload || payload->size() < 4) return {};
  const uint32_t units =
      static_cast<uint32_t>((*payload)[0]) | (static_cast<uint32_t>((*payload)[1]) << 8) |
      (static_cast<uint32_t>((*payload)[2]) << 16) | (static_cast<uint32_t>((*payload)[3]) << 24);
  const size_t bytes = static_cast<size_t>(units) * sizeof(uint16_t);
  if (units == 0 || payload->size() < 4 + bytes) return {};

  std::wstring text;
  text.reserve(units);
  const BYTE* data = payload->data() + 4;
  for (uint32_t i = 0; i < units; ++i) {
    const uint16_t unit = static_cast<uint16_t>(data[i * 2]) |
                          (static_cast<uint16_t>(data[i * 2 + 1]) << 8);
    if (unit == 0) break;
    text.push_back(static_cast<wchar_t>(unit));
  }
  return text;
}

bool ReadPayloadU32(const std::vector<BYTE>& payload, size_t* offset, uint32_t* value) {
  if (!offset || !value || *offset + 4 > payload.size()) return false;
  *value = static_cast<uint32_t>(payload[*offset]) |
           (static_cast<uint32_t>(payload[*offset + 1]) << 8) |
           (static_cast<uint32_t>(payload[*offset + 2]) << 16) |
           (static_cast<uint32_t>(payload[*offset + 3]) << 24);
  *offset += 4;
  return true;
}

bool ReadPayloadU16(const std::vector<BYTE>& payload, size_t* offset, uint16_t* value) {
  if (!offset || !value || *offset + 2 > payload.size()) return false;
  *value = static_cast<uint16_t>(payload[*offset]) |
           (static_cast<uint16_t>(payload[*offset + 1]) << 8);
  *offset += 2;
  return true;
}

bool ReadPayloadUtf16Units(const std::vector<BYTE>& payload, size_t* offset, size_t units,
                           size_t maxUnits, std::wstring* value) {
  if (!offset || !value || units > maxUnits) return false;
  const size_t bytes = units * sizeof(uint16_t);
  if (*offset + bytes > payload.size()) return false;
  value->clear();
  value->reserve(units);
  for (size_t i = 0; i < units; ++i) {
    const size_t pos = *offset + i * sizeof(uint16_t);
    const uint16_t unit =
        static_cast<uint16_t>(payload[pos]) | (static_cast<uint16_t>(payload[pos + 1]) << 8);
    if (unit == 0) break;
    value->push_back(static_cast<wchar_t>(unit));
  }
  *offset += bytes;
  return true;
}

bool ReadPayloadString(const std::vector<BYTE>& payload, size_t* offset, std::wstring* value) {
  uint32_t units = 0;
  if (!ReadPayloadU32(payload, offset, &units)) return false;
  const size_t bytes = static_cast<size_t>(units) * sizeof(uint16_t);
  if (!offset || !value || *offset + bytes > payload.size()) return false;
  value->clear();
  value->reserve(units);
  for (uint32_t i = 0; i < units; ++i) {
    const size_t pos = *offset + static_cast<size_t>(i) * 2;
    const uint16_t unit = static_cast<uint16_t>(payload[pos]) |
                          (static_cast<uint16_t>(payload[pos + 1]) << 8);
    if (unit == 0) break;
    value->push_back(static_cast<wchar_t>(unit));
  }
  *offset += bytes;
  return true;
}

bool ParseCompactLookupResponse(const std::vector<BYTE>& response,
                                std::vector<std::wstring>* candidates,
                                std::vector<std::wstring>* metaScores, int* count,
                                std::wstring* error) {
  if (!candidates || !count) return false;
  candidates->clear();
  if (metaScores) metaScores->clear();
  *count = 0;

  size_t offset = 0;
  uint32_t rawCount = 0;
  if (!ReadPayloadU32(response, &offset, &rawCount)) {
    if (error) *error = L"shared engine compact lookup response was truncated";
    return false;
  }
  if (rawCount > kRustRows) {
    if (error) *error = L"shared engine compact lookup response contained too many candidates";
    return false;
  }

  candidates->reserve(rawCount);
  if (metaScores) metaScores->reserve(rawCount);
  for (uint32_t i = 0; i < rawCount; ++i) {
    uint16_t textUnits = 0;
    if (!ReadPayloadU16(response, &offset, &textUnits)) {
      if (error) *error = L"shared engine compact lookup response text length was truncated";
      return false;
    }
    std::wstring text;
    if (!ReadPayloadUtf16Units(response, &offset, textUnits, kRustRowTextUnits, &text)) {
      if (error) *error = L"shared engine compact lookup response text was truncated";
      return false;
    }
    uint16_t metaUnits = 0;
    if (!ReadPayloadU16(response, &offset, &metaUnits)) {
      if (error) *error = L"shared engine compact lookup response meta length was truncated";
      return false;
    }
    std::wstring meta;
    if (!ReadPayloadUtf16Units(response, &offset, metaUnits, kRustRowMetaUnits, &meta)) {
      if (error) *error = L"shared engine compact lookup response meta was truncated";
      return false;
    }
    candidates->push_back(std::move(text));
    if (metaScores) metaScores->push_back(std::move(meta));
  }

  if (offset != response.size()) {
    if (error) *error = L"shared engine compact lookup response contained trailing bytes";
    return false;
  }
  *count = static_cast<int>(candidates->size());
  return true;
}

std::wstring RemoteStatusError(EnginePipeCommand command, int status,
                               const std::vector<BYTE>* payload = nullptr) {
  std::wstring message = L"shared engine ";
  message += RemoteCommandLabel(command);
  if (status == kRustFfiPanicRc) {
    message +=
        L" failed because the helper process panicked; details may be in %LOCALAPPDATA%\\kaixin\\logs\\engine.log";
    return message;
  }
  const std::wstring detail = DecodeRemoteErrorPayload(payload);
  if (!detail.empty()) {
    message += L" failed: ";
    message += detail;
    message += L" (rc=";
    message += std::to_wstring(status);
    message += L")";
    return message;
  }
  message += L" failed, rc=";
  message += std::to_wstring(status);
  return message;
}

bool IsPipeDisconnectSystemError(DWORD error) {
  return error == ERROR_NO_DATA || error == ERROR_BROKEN_PIPE;
}

bool IsTransientRemoteLookupError(const std::wstring& error) {
  return error.find(L" timed out after ") != std::wstring::npos ||
         error.find(L" ERROR_OPERATION_ABORTED") != std::wstring::npos ||
         error.find(L" [pipe-disconnect]") != std::wstring::npos;
}

bool IsLookupTimeoutError(const std::wstring& error) {
  return error.find(L"lookup") != std::wstring::npos &&
         error.find(L" timed out after ") != std::wstring::npos;
}

bool IsRemoteEngineBusyError(const std::wstring& error) {
  return error.find(L"shared engine busy") != std::wstring::npos;
}

void PublishLatestLookupRequestId(unsigned long long requestId) {
  if (requestId == 0) return;
  const ULONGLONG now = GetTickCount64();
  g_lastEngineUseTime.store(now, std::memory_order_release);
  g_lastInteractiveLookupTick.store(now, std::memory_order_release);
  unsigned long long current = g_latestLookupRequestId.load(std::memory_order_acquire);
  while (requestId > current &&
         !g_latestLookupRequestId.compare_exchange_weak(current, requestId,
                                                        std::memory_order_acq_rel,
                                                        std::memory_order_acquire)) {
  }
}

bool IsLookupRequestSuperseded(unsigned long long requestId) {
  return requestId != 0 &&
         g_latestLookupRequestId.load(std::memory_order_acquire) > requestId;
}

bool IsLookupSupersededError(const std::wstring& error) {
  return error.find(L"lookup superseded") != std::wstring::npos;
}

void ResetLookupTimeoutStreak() {
  g_consecutiveLookupTimeouts.store(0, std::memory_order_release);
}

bool ShouldRestartHelperAfterLookupTimeout() {
  const unsigned streak =
      g_consecutiveLookupTimeouts.fetch_add(1, std::memory_order_acq_rel) + 1;
  if (streak < kLookupTimeoutRestartThreshold) return false;

  const ULONGLONG now = GetTickCount64();
  ULONGLONG last = g_lastLookupTimeoutRestartTick.load(std::memory_order_acquire);
  while (last == 0 || now < last || now - last >= kLookupTimeoutRestartCooldownMs) {
    if (g_lastLookupTimeoutRestartTick.compare_exchange_weak(last, now, std::memory_order_acq_rel,
                                                             std::memory_order_acquire)) {
      ResetLookupTimeoutStreak();
      return true;
    }
  }
  return false;
}

DWORD TimeoutMsForCommand(EnginePipeCommand command) {
  switch (command) {
    case EnginePipeCommand::Init:
      return kEnginePipeInitTimeoutMs;
    case EnginePipeCommand::Lookup:
      return kEnginePipeLookupTimeoutMs;
    case EnginePipeCommand::Learn:
      return kEnginePipeLearnTimeoutMs;
    case EnginePipeCommand::SyllableBounds:
      return kEnginePipeSyllableTimeoutMs;
    case EnginePipeCommand::RecordClipboard:
      return kEnginePipeClipboardTimeoutMs;
    case EnginePipeCommand::ResolveClipboard:
      return kEnginePipeClipboardTimeoutMs;
    case EnginePipeCommand::SetCandidatePin:
    case EnginePipeCommand::CandidateAction:
    case EnginePipeCommand::ResetLearningContext:
      return kEnginePipeLearnTimeoutMs;
    case EnginePipeCommand::LearnCorrection:
    case EnginePipeCommand::LearnSelectionFeedback:
      return kEnginePipeLearnTimeoutMs;
    case EnginePipeCommand::Health:
      return kEnginePipeHealthTimeoutMs;
    case EnginePipeCommand::Shutdown:
      return kEnginePipeShutdownTimeoutMs;
    case EnginePipeCommand::CancelLookup:
      return kEnginePipeCancelTimeoutMs;
  }
  return kEnginePipeLookupTimeoutMs;
}

DWORD RetryTimeoutMsForCommand(EnginePipeCommand command, DWORD timeoutMs) {
  switch (command) {
    case EnginePipeCommand::Lookup:
      return kEnginePipeLookupRetryTimeoutMs;
    case EnginePipeCommand::Init:
      return timeoutMs;
    case EnginePipeCommand::Learn:
    case EnginePipeCommand::SyllableBounds:
    case EnginePipeCommand::RecordClipboard:
    case EnginePipeCommand::ResolveClipboard:
    case EnginePipeCommand::SetCandidatePin:
    case EnginePipeCommand::CandidateAction:
    case EnginePipeCommand::ResetLearningContext:
    case EnginePipeCommand::LearnCorrection:
    case EnginePipeCommand::LearnSelectionFeedback:
    case EnginePipeCommand::Health:
    case EnginePipeCommand::Shutdown:
    case EnginePipeCommand::CancelLookup:
      return timeoutMs;
  }
  return timeoutMs;
}

void CloseRemotePipeOnlyLocked() {
  if (g_bridge.pipe != nullptr && g_bridge.pipe != INVALID_HANDLE_VALUE) {
    CloseHandle(g_bridge.pipe);
    g_bridge.pipe = INVALID_HANDLE_VALUE;
  }
}

void InvalidateRemoteBackendLocked() {
  ClearLocalLookupCache();
  SetLocalLookupCacheSignature(std::wstring());
  CloseRemotePipeOnlyLocked();
  if (g_bridge.backend == EngineBackend::Remote) {
    g_bridge.backend = EngineBackend::None;
    g_bridge.initialized = false;
    g_bridge.loadedLexiconDir.clear();
    g_bridge.buildId.clear();
    g_bridge.cacheSignature.clear();
  }
}

bool AwaitPipeIo(HANDLE handle, OVERLAPPED* overlapped, DWORD timeoutMs, DWORD* transferred,
                 EnginePipeCommand command, const wchar_t* stage, std::wstring* error) {
  const DWORD wait = WaitForSingleObject(overlapped->hEvent, timeoutMs);
  if (wait == WAIT_OBJECT_0) {
    if (GetOverlappedResult(handle, overlapped, transferred, FALSE)) return true;
    if (error) {
      *error = L"shared engine pipe ";
      *error += RemoteCommandLabel(command);
      *error += L" ";
      *error += stage;
      *error += L" failed: ";
      *error += FormatSystemError(GetLastError());
    }
    return false;
  }

  if (wait == WAIT_TIMEOUT) {
    CancelIoEx(handle, overlapped);
    DWORD ignored = 0;
    const DWORD cancelWait = WaitForSingleObject(overlapped->hEvent, 200);
    if (cancelWait == WAIT_OBJECT_0) {
      (void)GetOverlappedResult(handle, overlapped, &ignored, FALSE);
    }
    if (error) {
      *error = L"shared engine pipe ";
      *error += RemoteCommandLabel(command);
      *error += L" ";
      *error += stage;
      *error += L" timed out after ";
      *error += std::to_wstring(timeoutMs);
      *error += L" ms";
    }
    return false;
  }

  if (error) {
    *error = L"shared engine pipe ";
    *error += RemoteCommandLabel(command);
    *error += L" wait failed: ";
    *error += FormatSystemError(GetLastError());
  }
  return false;
}

bool WritePipeMessageWithTimeout(HANDLE handle, const void* buffer, size_t bytes, DWORD timeoutMs,
                                 EnginePipeCommand command, const wchar_t* stage,
                                 std::wstring* error) {
  if (!buffer || bytes == 0) return true;

  HANDLE event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (!event) {
    if (error) {
      *error = L"CreateEventW failed for shared engine pipe write: ";
      *error += FormatSystemError(GetLastError());
    }
    return false;
  }

  OVERLAPPED overlapped = {};
  overlapped.hEvent = event;

  DWORD transferred = 0;
  const BOOL ok = WriteFile(handle, buffer, static_cast<DWORD>(bytes), &transferred, &overlapped);
  if (!ok) {
    const DWORD lastError = GetLastError();
    if (lastError != ERROR_IO_PENDING) {
      CloseHandle(event);
      if (error) {
        *error = L"shared engine pipe ";
        *error += RemoteCommandLabel(command);
        *error += L" ";
        *error += stage;
        *error += L" write failed: ";
        *error += FormatSystemError(lastError);
        if (IsPipeDisconnectSystemError(lastError)) *error += L" [pipe-disconnect]";
      }
      return false;
    }

    if (!AwaitPipeIo(handle, &overlapped, timeoutMs, &transferred, command, stage, error)) {
      CloseHandle(event);
      return false;
    }
  }

  CloseHandle(event);
  if (transferred != bytes) {
    if (error) {
      *error = L"shared engine pipe ";
      *error += RemoteCommandLabel(command);
      *error += L" ";
      *error += stage;
      *error += L" write was truncated";
    }
    return false;
  }
  return true;
}

bool ReadPipeMessageWithTimeout(HANDLE handle, void* buffer, size_t bytes, DWORD timeoutMs,
                                EnginePipeCommand command, const wchar_t* stage, std::wstring* error) {
  if (!buffer || bytes == 0) return true;

  HANDLE event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (!event) {
    if (error) {
      *error = L"CreateEventW failed for shared engine pipe read: ";
      *error += FormatSystemError(GetLastError());
    }
    return false;
  }

  OVERLAPPED overlapped = {};
  overlapped.hEvent = event;

  DWORD transferred = 0;
  const BOOL ok = ReadFile(handle, buffer, static_cast<DWORD>(bytes), &transferred, &overlapped);
  if (!ok) {
    const DWORD lastError = GetLastError();
    if (lastError != ERROR_IO_PENDING) {
      CloseHandle(event);
      if (error) {
        *error = L"shared engine pipe ";
        *error += RemoteCommandLabel(command);
        *error += L" ";
        *error += stage;
        *error += L" read failed: ";
        *error += FormatSystemError(lastError);
        if (IsPipeDisconnectSystemError(lastError)) *error += L" [pipe-disconnect]";
      }
      return false;
    }

    if (!AwaitPipeIo(handle, &overlapped, timeoutMs, &transferred, command, stage, error)) {
      CloseHandle(event);
      return false;
    }
  }

  CloseHandle(event);
  if (transferred != bytes) {
    if (error) {
      *error = L"shared engine pipe ";
      *error += RemoteCommandLabel(command);
      *error += L" ";
      *error += stage;
      *error += L" response was truncated";
    }
    return false;
  }
  return true;
}

bool EnsureRemoteConnectionLocked(EnginePipeCommand command, DWORD timeoutMs, std::wstring* error) {
  if (g_bridge.pipe != nullptr && g_bridge.pipe != INVALID_HANDLE_VALUE) return true;

  const std::wstring pipeName = EnginePipeNameForModuleDir(ModuleDir());
  const ULONGLONG deadline = GetTickCount64() + timeoutMs;
  while (true) {
    HANDLE pipe = CreateFileW(pipeName.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING,
                              FILE_FLAG_OVERLAPPED, nullptr);
    if (pipe != INVALID_HANDLE_VALUE) {
      DWORD mode = PIPE_READMODE_MESSAGE;
      if (!SetNamedPipeHandleState(pipe, &mode, nullptr, nullptr)) {
        if (error) *error = L"SetNamedPipeHandleState failed: " + FormatSystemError(GetLastError());
        CloseHandle(pipe);
        return false;
      }
      g_bridge.pipe = pipe;
      return true;
    }

    const DWORD lastError = GetLastError();
    if (lastError != ERROR_FILE_NOT_FOUND && lastError != ERROR_PIPE_BUSY) {
      if (error) *error = L"CreateFileW(pipe) failed: " + FormatSystemError(lastError);
      return false;
    }

    if (GetTickCount64() >= deadline) {
      if (error) {
        *error = L"shared engine pipe ";
        *error += RemoteCommandLabel(command);
        *error += L" connection timed out after ";
        *error += std::to_wstring(timeoutMs);
        *error += L" ms";
      }
      return false;
    }

    if (lastError == ERROR_PIPE_BUSY) {
      const ULONGLONG remaining64 =
          deadline > GetTickCount64() ? (deadline - GetTickCount64()) : 0ULL;
      const DWORD remaining = static_cast<DWORD>(
          remaining64 < static_cast<ULONGLONG>(kEnginePipeRetrySleepMs) ? remaining64
                                                                        : kEnginePipeRetrySleepMs);
      if (!WaitNamedPipeW(pipeName.c_str(), remaining == 0 ? 1 : remaining)) {
        Sleep(remaining == 0 ? 1 : remaining);
      }
    } else {
      const ULONGLONG remaining64 =
          deadline > GetTickCount64() ? (deadline - GetTickCount64()) : 0ULL;
      const DWORD remaining = static_cast<DWORD>(
          remaining64 < static_cast<ULONGLONG>(kEnginePipeRetrySleepMs) ? remaining64
                                                                        : kEnginePipeRetrySleepMs);
      Sleep(remaining == 0 ? 1 : remaining);
    }
  }
}

bool SendRemoteRequestLocked(EnginePipeCommand command, const std::vector<BYTE>& payload, int* status,
                             std::vector<BYTE>* responsePayload, std::wstring* error) {
  if (status) *status = -1;
  if (responsePayload) responsePayload->clear();
  const DWORD timeoutMs = TimeoutMsForCommand(command);
  if (!EnsureRemoteConnectionLocked(command, timeoutMs, error)) return false;

  std::vector<BYTE> header;
  header.reserve(12);
  AppendU32(header, kEnginePipeMagic);
  AppendU16(header, kEnginePipeVersion);
  AppendU16(header, static_cast<uint16_t>(command));
  AppendU32(header, static_cast<uint32_t>(payload.size()));
  if (!WritePipeMessageWithTimeout(g_bridge.pipe, header.data(), header.size(), timeoutMs, command,
                                   L"request-header", error) ||
      (!payload.empty() && !WritePipeMessageWithTimeout(g_bridge.pipe, payload.data(), payload.size(),
                                                        timeoutMs, command, L"request-payload", error))) {
    InvalidateRemoteBackendLocked();
    return false;
  }

  BYTE responseHeader[kEnginePipeResponseHeaderBytes] = {};
  bool readOk = ReadPipeMessageWithTimeout(g_bridge.pipe, responseHeader, sizeof(responseHeader),
                                           timeoutMs, command, L"response-header", error);
  if (!readOk && (command == EnginePipeCommand::Lookup || command == EnginePipeCommand::Init)) {
    // Retry once with a wider timeout budget. Lookup can be busy in the
    // engine; Init can contend with lexicon loading or warmup.
    if (error) error->clear();
    const DWORD retryTimeoutMs = RetryTimeoutMsForCommand(command, timeoutMs);
    readOk = ReadPipeMessageWithTimeout(g_bridge.pipe, responseHeader, sizeof(responseHeader),
                                        retryTimeoutMs, command, L"response-header(retry)", error);
  }
  if (!readOk) {
    InvalidateRemoteBackendLocked();
    return false;
  }

  const uint32_t magic =
      static_cast<uint32_t>(responseHeader[0]) | (static_cast<uint32_t>(responseHeader[1]) << 8) |
      (static_cast<uint32_t>(responseHeader[2]) << 16) | (static_cast<uint32_t>(responseHeader[3]) << 24);
  const uint16_t version = static_cast<uint16_t>(responseHeader[4]) |
                           (static_cast<uint16_t>(responseHeader[5]) << 8);
  const uint16_t echoedCommand = static_cast<uint16_t>(responseHeader[6]) |
                                 (static_cast<uint16_t>(responseHeader[7]) << 8);
  const int32_t responseStatus =
      static_cast<int32_t>(static_cast<uint32_t>(responseHeader[8]) |
                           (static_cast<uint32_t>(responseHeader[9]) << 8) |
                           (static_cast<uint32_t>(responseHeader[10]) << 16) |
                           (static_cast<uint32_t>(responseHeader[11]) << 24));
  const uint32_t responseSize =
      static_cast<uint32_t>(responseHeader[12]) | (static_cast<uint32_t>(responseHeader[13]) << 8) |
      (static_cast<uint32_t>(responseHeader[14]) << 16) | (static_cast<uint32_t>(responseHeader[15]) << 24);

  if (responseSize > kEnginePipeMaxResponseBytes) {
    if (error) *error = L"shared engine pipe returned an oversized response payload";
    InvalidateRemoteBackendLocked();
    return false;
  }

  if (magic != kEnginePipeMagic || version != kEnginePipeVersion ||
      echoedCommand != static_cast<uint16_t>(command)) {
    if (error) *error = L"shared engine pipe returned an invalid response header";
    InvalidateRemoteBackendLocked();
    return false;
  }

  if (responsePayload && responseSize > 0) {
    responsePayload->resize(responseSize);
    if (!ReadPipeMessageWithTimeout(g_bridge.pipe, responsePayload->data(), responsePayload->size(),
                                    timeoutMs, command, L"response-payload", error)) {
      InvalidateRemoteBackendLocked();
      return false;
    }
  } else if (responseSize > 0) {
    std::vector<BYTE> sink(responseSize);
    if (!ReadPipeMessageWithTimeout(g_bridge.pipe, sink.data(), sink.size(), timeoutMs, command,
                                    L"response-payload", error)) {
      InvalidateRemoteBackendLocked();
      return false;
    }
  }

  if (status) *status = responseStatus;
  if (command == EnginePipeCommand::Shutdown) CloseRemotePipeOnlyLocked();
  return true;
}

bool SendLookupCancelRequest(unsigned long long supersedingRequestId) {
  if (supersedingRequestId == 0) return false;
  const std::wstring pipeName = EnginePipeNameForModuleDir(ModuleDir());
  HANDLE pipe = INVALID_HANDLE_VALUE;
  for (int attempt = 0; attempt < 2; ++attempt) {
    pipe = CreateFileW(pipeName.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING,
                       FILE_FLAG_OVERLAPPED, nullptr);
    if (pipe != INVALID_HANDLE_VALUE) break;
    if (GetLastError() != ERROR_PIPE_BUSY || attempt != 0) return false;
    if (!WaitNamedPipeW(pipeName.c_str(), kEnginePipeCancelTimeoutMs)) return false;
  }
  if (pipe == INVALID_HANDLE_VALUE) return false;

  auto closePipe = [&]() {
    if (pipe != INVALID_HANDLE_VALUE) {
      CloseHandle(pipe);
      pipe = INVALID_HANDLE_VALUE;
    }
  };
  DWORD mode = PIPE_READMODE_MESSAGE;
  if (!SetNamedPipeHandleState(pipe, &mode, nullptr, nullptr)) {
    closePipe();
    return false;
  }

  std::vector<BYTE> payload;
  payload.reserve(sizeof(uint64_t));
  AppendU64(payload, static_cast<uint64_t>(supersedingRequestId));
  std::vector<BYTE> header;
  header.reserve(12);
  AppendU32(header, kEnginePipeMagic);
  AppendU16(header, kEnginePipeVersion);
  AppendU16(header, static_cast<uint16_t>(EnginePipeCommand::CancelLookup));
  AppendU32(header, static_cast<uint32_t>(payload.size()));

  std::wstring error;
  if (!WritePipeMessageWithTimeout(pipe, header.data(), header.size(), kEnginePipeCancelTimeoutMs,
                                   EnginePipeCommand::CancelLookup, L"request-header", &error) ||
      !WritePipeMessageWithTimeout(pipe, payload.data(), payload.size(), kEnginePipeCancelTimeoutMs,
                                   EnginePipeCommand::CancelLookup, L"request-payload", &error)) {
    closePipe();
    return false;
  }

  BYTE responseHeader[kEnginePipeResponseHeaderBytes] = {};
  if (!ReadPipeMessageWithTimeout(pipe, responseHeader, sizeof(responseHeader),
                                  kEnginePipeCancelTimeoutMs, EnginePipeCommand::CancelLookup,
                                  L"response-header", &error)) {
    closePipe();
    return false;
  }
  closePipe();
  const uint32_t magic =
      static_cast<uint32_t>(responseHeader[0]) | (static_cast<uint32_t>(responseHeader[1]) << 8) |
      (static_cast<uint32_t>(responseHeader[2]) << 16) |
      (static_cast<uint32_t>(responseHeader[3]) << 24);
  const uint16_t version = static_cast<uint16_t>(responseHeader[4]) |
                           (static_cast<uint16_t>(responseHeader[5]) << 8);
  const uint16_t command = static_cast<uint16_t>(responseHeader[6]) |
                           (static_cast<uint16_t>(responseHeader[7]) << 8);
  const int32_t status =
      static_cast<int32_t>(static_cast<uint32_t>(responseHeader[8]) |
                           (static_cast<uint32_t>(responseHeader[9]) << 8) |
                           (static_cast<uint32_t>(responseHeader[10]) << 16) |
                           (static_cast<uint32_t>(responseHeader[11]) << 24));
  const uint32_t responseSize =
      static_cast<uint32_t>(responseHeader[12]) |
      (static_cast<uint32_t>(responseHeader[13]) << 8) |
      (static_cast<uint32_t>(responseHeader[14]) << 16) |
      (static_cast<uint32_t>(responseHeader[15]) << 24);
  return magic == kEnginePipeMagic && version == kEnginePipeVersion &&
         command == static_cast<uint16_t>(EnginePipeCommand::CancelLookup) && status == 0 &&
         responseSize == 0;
}

void LookupCancelWorkerMain() {
  for (;;) {
    const unsigned long long target =
        g_latestLookupCancelRequestId.load(std::memory_order_acquire);
    (void)SendLookupCancelRequest(target);
    if (g_latestLookupCancelRequestId.load(std::memory_order_acquire) != target) continue;

    g_lookupCancelWorkerRunning.store(false, std::memory_order_release);
    if (g_latestLookupCancelRequestId.load(std::memory_order_acquire) == target) return;

    bool expected = false;
    if (!g_lookupCancelWorkerRunning.compare_exchange_strong(
            expected, true, std::memory_order_acq_rel, std::memory_order_acquire)) {
      return;
    }
  }
}

bool RequestRemoteHelperShutdownLocked(std::wstring* error) {
  std::vector<BYTE> payload;
  if (!AppendCapabilityToken(payload, error)) return false;
  int status = -1;
  std::vector<BYTE> response;
  if (!SendRemoteRequestLocked(EnginePipeCommand::Shutdown, payload, &status, &response, error)) {
    InvalidateRemoteBackendLocked();
    return false;
  }
  InvalidateRemoteBackendLocked();
  if (status != 0) {
    if (error) *error = RemoteStatusError(EnginePipeCommand::Shutdown, status, &response);
    return false;
  }
  return true;
}

bool EnsureRemoteLoadedLocked(const std::filesystem::path& lexiconDir, std::wstring* error) {
  if (g_bridge.initialized && g_bridge.backend == EngineBackend::Remote &&
      PathEquals(g_bridge.loadedLexiconDir, lexiconDir)) {
    return true;
  }

  std::vector<BYTE> payload;
  payload.reserve(4 + lexiconDir.wstring().size() * sizeof(uint16_t));
  AppendWideString(payload, lexiconDir.wstring());

  int status = -1;
  std::vector<BYTE> response;
  if (!SendRemoteRequestLocked(EnginePipeCommand::Init, payload, &status, &response, error)) {
    return false;
  }
  if (status != 0) {
    if (error) *error = RemoteStatusError(EnginePipeCommand::Init, status, &response);
    InvalidateRemoteBackendLocked();
    return false;
  }

  g_bridge.backend = EngineBackend::Remote;
  g_bridge.initialized = true;
  g_bridge.loadedLexiconDir = lexiconDir;
  return true;
}

bool VerifyRemoteInstanceLocked(const std::filesystem::path& moduleDir, std::wstring* error) {
  int status = -1;
  std::vector<BYTE> response;
  if (!SendRemoteRequestLocked(EnginePipeCommand::Health, {}, &status, &response, error)) {
    return false;
  }
  if (status != 0) {
    if (error) *error = RemoteStatusError(EnginePipeCommand::Health, status, &response);
    InvalidateRemoteBackendLocked();
    return false;
  }

  uint32_t version = 0;
  uint32_t loaded = 0;
  std::wstring buildId;
  std::wstring helperExe;
  std::wstring installRoot;
  std::wstring loadedLexicon;
  std::wstring pipeName;
  std::wstring mutexName;
  std::wstring lexiconState;
  std::wstring targetLexicon;
  std::wstring modelHash;
  std::wstring cacheSignature;
  size_t offset = 0;
  const bool parsed = ReadPayloadU32(response, &offset, &version) &&
                      ReadPayloadString(response, &offset, &buildId) &&
                      ReadPayloadString(response, &offset, &helperExe) &&
                      ReadPayloadString(response, &offset, &installRoot) &&
                      ReadPayloadString(response, &offset, &loadedLexicon) &&
                      ReadPayloadString(response, &offset, &pipeName) &&
                      ReadPayloadString(response, &offset, &mutexName) &&
                      ReadPayloadU32(response, &offset, &loaded);
  if (!parsed || version != kEnginePipeVersion) {
    if (error) *error = L"shared engine health response was invalid";
    InvalidateRemoteBackendLocked();
    return false;
  }

  uint32_t fullInFlight = 0;
  if (offset < response.size()) {
    if (!ReadPayloadString(response, &offset, &lexiconState) ||
        !ReadPayloadString(response, &offset, &targetLexicon) ||
        !ReadPayloadU32(response, &offset, &fullInFlight) ||
        !ReadPayloadString(response, &offset, &modelHash) ||
        !ReadPayloadString(response, &offset, &cacheSignature)) {
      if (error) *error = L"shared engine health response was truncated";
      InvalidateRemoteBackendLocked();
      return false;
    }
  }

  const std::wstring expectedBuildId = ExpectedEngineBuildId();
  // Git revision differences should invalidate caches and leave a log trail, but
  // protocol/app-version compatibility is enough for the shared helper handshake.
  if (!EngineBuildIdHasExpectedVersion(buildId)) {
    std::wstring mismatch = L"shared engine helper build id version mismatch: helper=";
    mismatch += buildId.empty() ? L"(empty)" : buildId;
    mismatch += L", expected=";
    mismatch += expectedBuildId;
    std::wstring shutdownError;
    if (!RequestRemoteHelperShutdownLocked(&shutdownError) && !shutdownError.empty()) {
      mismatch += L"; shutdown=";
      mismatch += shutdownError;
    }
    if (error) *error = mismatch;
    InvalidateRemoteBackendLocked();
    return false;
  }
  if (_wcsicmp(buildId.c_str(), expectedBuildId.c_str()) != 0) {
    std::wstring mismatch = L"shared engine helper build id differs; accepting compatible helper: helper=";
    mismatch += buildId;
    mismatch += L", expected=";
    mismatch += expectedBuildId;
    AppendEngineStateLogDeduped(mismatch);
  }

  const std::wstring expectedPipeName = EnginePipeNameForModuleDir(moduleDir);
  const std::wstring expectedMutexName = EngineMutexNameForModuleDir(moduleDir);
  if ((!pipeName.empty() && _wcsicmp(pipeName.c_str(), expectedPipeName.c_str()) != 0) ||
      (!mutexName.empty() && _wcsicmp(mutexName.c_str(), expectedMutexName.c_str()) != 0)) {
    if (error) {
      *error = L"shared engine helper IPC identity mismatch: helper_pipe=";
      *error += pipeName;
      *error += L", expected_pipe=";
      *error += expectedPipeName;
      *error += L", helper_mutex=";
      *error += mutexName;
      *error += L", expected_mutex=";
      *error += expectedMutexName;
    }
    InvalidateRemoteBackendLocked();
    return false;
  }

  std::filesystem::path expectedInstallRoot;
  if (ResolveTrustedInstallLayout(moduleDir, &expectedInstallRoot, nullptr) &&
      !expectedInstallRoot.empty() && !installRoot.empty() &&
      !PathStartsWith(std::filesystem::path(installRoot), expectedInstallRoot)) {
    if (error) {
      *error = L"shared engine helper install root mismatch: helper=";
      *error += installRoot;
      *error += L", tip=";
      *error += expectedInstallRoot.wstring();
    }
    InvalidateRemoteBackendLocked();
    return false;
  }
  if (_wcsicmp(lexiconState.c_str(), L"busy") == 0) {
    g_bridge.buildId = buildId;
    AppendEngineStateLogDeduped(L"shared helper health busy: build=" + buildId);
    return true;
  }
  if (cacheSignature.empty()) {
    cacheSignature = L"build=" + buildId + L";model=" + modelHash + L";lexicon_state=" +
                     lexiconState + L";lexicon=" + targetLexicon;
  }
  g_bridge.buildId = buildId;
  g_bridge.cacheSignature = cacheSignature;
  SetLocalLookupCacheSignature(cacheSignature);
  AppendEngineStateLogDeduped(L"shared helper health ok: build=" + buildId +
                              L"; lexicon_state=" + lexiconState +
                              L"; full_in_flight=" + std::to_wstring(fullInFlight));
  return true;
}

bool RemoteLookupLocked(const std::wstring& reading, unsigned long long requestId,
                        std::vector<std::wstring>* candidates,
                        std::vector<std::wstring>* metaScores, int* count,
                        std::wstring* error) {
  if (!candidates || !count) return false;
  candidates->clear();
  if (metaScores) metaScores->clear();
  *count = 0;

  std::vector<BYTE>& payload = RemoteLookupPayloadScratch();
  payload.clear();
  payload.reserve(96 + reading.size() * sizeof(uint16_t));
  if (!AppendCapabilityToken(payload, error)) return false;
  AppendU32(payload, g_bridge.pendingModeFlags);
  AppendWideString(payload, reading);
  AppendU64(payload, static_cast<uint64_t>(requestId));

  int status = -1;
  std::vector<BYTE>& response = RemoteLookupResponseScratch();
  if (!SendRemoteRequestLocked(EnginePipeCommand::Lookup, payload, &status, &response, error)) {
    return false;
  }
  if (status != 0) {
    if (error) *error = RemoteStatusError(EnginePipeCommand::Lookup, status, &response);
    return false;
  }
  if (response.size() < 4) {
    if (error) *error = L"shared engine lookup response was truncated";
    return false;
  }

  std::wstring parseError;
  if (ParseCompactLookupResponse(response, candidates, metaScores, count, &parseError)) return true;
  candidates->clear();
  if (metaScores) metaScores->clear();
  *count = 0;
  if (error) {
    *error = parseError;
    if (error->empty()) *error = L"shared engine lookup response could not be parsed";
  }
  return false;
}

bool RemoteLearnLocked(const std::wstring& reading, const std::wstring& committedText,
                       unsigned long flags, std::wstring* error) {
  std::vector<BYTE> payload;
  payload.reserve(80 + 12 + (reading.size() + committedText.size()) * sizeof(uint16_t));
  if (!AppendCapabilityToken(payload, error)) return false;
  AppendU32(payload, static_cast<uint32_t>(reading.size()));
  AppendU32(payload, static_cast<uint32_t>(committedText.size()));
  for (wchar_t ch : reading) AppendU16(payload, static_cast<uint16_t>(ch));
  for (wchar_t ch : committedText) AppendU16(payload, static_cast<uint16_t>(ch));
  if (flags != kSrfLearnCommitDefault) AppendU32(payload, static_cast<uint32_t>(flags));

  int status = -1;
  std::vector<BYTE> response;
  if (!SendRemoteRequestLocked(EnginePipeCommand::Learn, payload, &status, &response, error)) {
    return false;
  }
  if (status != 0) {
    if (error) *error = RemoteStatusError(EnginePipeCommand::Learn, status, &response);
    return false;
  }
  return true;
}

bool RemoteLearnCorrectionLocked(const std::wstring& rawReading,
                                 const std::wstring& correctedReading,
                                 const std::wstring& committedText, std::wstring* error) {
  std::vector<BYTE> payload;
  payload.reserve(80 + 16 + (rawReading.size() + correctedReading.size() + committedText.size()) *
                           sizeof(uint16_t));
  if (!AppendCapabilityToken(payload, error)) return false;
  AppendU32(payload, static_cast<uint32_t>(rawReading.size()));
  AppendU32(payload, static_cast<uint32_t>(correctedReading.size()));
  AppendU32(payload, static_cast<uint32_t>(committedText.size()));
  for (wchar_t ch : rawReading) AppendU16(payload, static_cast<uint16_t>(ch));
  for (wchar_t ch : correctedReading) AppendU16(payload, static_cast<uint16_t>(ch));
  for (wchar_t ch : committedText) AppendU16(payload, static_cast<uint16_t>(ch));

  int status = -1;
  std::vector<BYTE> response;
  if (!SendRemoteRequestLocked(EnginePipeCommand::LearnCorrection, payload, &status, &response,
                               error)) {
    return false;
  }
  if (status != 0) {
    if (error) {
      *error = RemoteStatusError(EnginePipeCommand::LearnCorrection, status, &response);
    }
    return false;
  }
  return true;
}

bool RemoteLearnSelectionFeedbackLocked(const std::wstring& reading,
                                        const std::wstring& committedText,
                                        unsigned long selectedIndex, unsigned long page,
                                        const std::vector<std::wstring>& skippedCandidates,
                                        std::wstring* error) {
  std::vector<const std::wstring*> skipped;
  skipped.reserve(skippedCandidates.size() < kMaxSelectionFeedbackSkipped
                      ? skippedCandidates.size()
                      : kMaxSelectionFeedbackSkipped);
  size_t skippedUnits = 0;
  for (const auto& candidate : skippedCandidates) {
    if (candidate.empty() || candidate == committedText || candidate.size() > kMaxLearnPhraseUnits) {
      continue;
    }
    skipped.push_back(&candidate);
    skippedUnits += candidate.size();
    if (skipped.size() >= kMaxSelectionFeedbackSkipped) break;
  }

  std::vector<BYTE> payload;
  payload.reserve(80 + 20 + (reading.size() + committedText.size() + skippedUnits) *
                           sizeof(uint16_t) +
                  skipped.size() * 4);
  if (!AppendCapabilityToken(payload, error)) return false;
  AppendU32(payload, static_cast<uint32_t>(reading.size()));
  AppendU32(payload, static_cast<uint32_t>(committedText.size()));
  AppendU32(payload, static_cast<uint32_t>(selectedIndex));
  AppendU32(payload, static_cast<uint32_t>(page));
  for (wchar_t ch : reading) AppendU16(payload, static_cast<uint16_t>(ch));
  for (wchar_t ch : committedText) AppendU16(payload, static_cast<uint16_t>(ch));
  AppendU32(payload, static_cast<uint32_t>(skipped.size()));
  for (const std::wstring* candidate : skipped) {
    AppendU32(payload, static_cast<uint32_t>(candidate->size()));
    for (wchar_t ch : *candidate) AppendU16(payload, static_cast<uint16_t>(ch));
  }

  int status = -1;
  std::vector<BYTE> response;
  if (!SendRemoteRequestLocked(EnginePipeCommand::LearnSelectionFeedback, payload, &status,
                               &response, error)) {
    return false;
  }
  if (status != 0) {
    if (error) {
      *error = RemoteStatusError(EnginePipeCommand::LearnSelectionFeedback, status, &response);
    }
    return false;
  }
  return true;
}

bool RemoteSetCandidatePinLocked(const std::wstring& reading, const std::wstring& committedText,
                                 bool pinned, std::wstring* error) {
  std::vector<BYTE> payload;
  payload.reserve(80 + 12 + (reading.size() + committedText.size()) * sizeof(uint16_t));
  if (!AppendCapabilityToken(payload, error)) return false;
  AppendU32(payload, pinned ? 1u : 0u);
  AppendU32(payload, static_cast<uint32_t>(reading.size()));
  AppendU32(payload, static_cast<uint32_t>(committedText.size()));
  for (wchar_t ch : reading) AppendU16(payload, static_cast<uint16_t>(ch));
  for (wchar_t ch : committedText) AppendU16(payload, static_cast<uint16_t>(ch));

  int status = -1;
  if (!SendRemoteRequestLocked(EnginePipeCommand::SetCandidatePin, payload, &status, nullptr,
                               error)) {
    return false;
  }
  if (status != 0) {
    if (error) *error = RemoteStatusError(EnginePipeCommand::SetCandidatePin, status);
    return false;
  }
  return true;
}

bool RemoteCandidateActionLocked(const std::wstring& reading, const std::wstring& committedText,
                                 SrfCandidateAction action, std::wstring* error) {
  std::vector<BYTE> payload;
  payload.reserve(80 + 12 + (reading.size() + committedText.size()) * sizeof(uint16_t));
  if (!AppendCapabilityToken(payload, error)) return false;
  AppendU32(payload, static_cast<uint32_t>(action));
  AppendU32(payload, static_cast<uint32_t>(reading.size()));
  AppendU32(payload, static_cast<uint32_t>(committedText.size()));
  for (wchar_t ch : reading) AppendU16(payload, static_cast<uint16_t>(ch));
  for (wchar_t ch : committedText) AppendU16(payload, static_cast<uint16_t>(ch));

  int status = -1;
  if (!SendRemoteRequestLocked(EnginePipeCommand::CandidateAction, payload, &status, nullptr,
                               error)) {
    return false;
  }
  if (status != 0) {
    if (error) *error = RemoteStatusError(EnginePipeCommand::CandidateAction, status);
    return false;
  }
  return true;
}

bool RemoteResetLearningContextLocked(std::wstring* error) {
  std::vector<BYTE> payload;
  payload.reserve(80);
  if (!AppendCapabilityToken(payload, error)) return false;

  int status = -1;
  if (!SendRemoteRequestLocked(EnginePipeCommand::ResetLearningContext, payload, &status, nullptr,
                               error)) {
    return false;
  }
  if (status != 0) {
    if (error) *error = RemoteStatusError(EnginePipeCommand::ResetLearningContext, status);
    return false;
  }
  return true;
}

bool RemoteRecordClipboardLocked(const std::wstring& text, std::wstring* error) {
  std::vector<BYTE> payload;
  payload.reserve(80 + 4 + text.size() * sizeof(uint16_t));
  if (!AppendCapabilityToken(payload, error)) return false;
  AppendWideString(payload, text);

  int status = -1;
  if (!SendRemoteRequestLocked(EnginePipeCommand::RecordClipboard, payload, &status, nullptr, error)) {
    return false;
  }
  if (status != 0) {
    if (error) *error = RemoteStatusError(EnginePipeCommand::RecordClipboard, status);
    return false;
  }
  return true;
}

bool RemoteResolveClipboardLocked(const std::wstring& id, std::wstring* text, std::wstring* error) {
  if (text) text->clear();

  std::vector<BYTE> payload;
  payload.reserve(80 + 4 + id.size() * sizeof(uint16_t));
  if (!AppendCapabilityToken(payload, error)) return false;
  AppendWideString(payload, id);

  int status = -1;
  std::vector<BYTE> response;
  if (!SendRemoteRequestLocked(EnginePipeCommand::ResolveClipboard, payload, &status, &response,
                               error)) {
    return false;
  }
  if (status != 0) {
    if (error) *error = RemoteStatusError(EnginePipeCommand::ResolveClipboard, status, &response);
    return false;
  }

  std::wstring resolved = DecodeRemoteErrorPayload(&response);
  if (resolved.empty()) {
    if (error) *error = L"shared engine clipboard resolve returned empty text";
    return false;
  }
  if (text) *text = std::move(resolved);
  return true;
}

bool RemoteSyllableBoundsLocked(const wchar_t* text, size_t unitCount, uint32_t* out, size_t cap, size_t* written,
                                std::wstring* error) {
  if (written) *written = 0;
  if (!text || !out || cap == 0) return false;

  std::vector<BYTE> payload;
  payload.reserve(4 + unitCount * sizeof(uint16_t));
  AppendU32(payload, static_cast<uint32_t>(unitCount));
  for (size_t i = 0; i < unitCount; ++i) {
    AppendU16(payload, static_cast<uint16_t>(text[i]));
  }

  int status = -1;
  std::vector<BYTE> response;
  if (!SendRemoteRequestLocked(EnginePipeCommand::SyllableBounds, payload, &status, &response, error)) {
    return false;
  }
  if (status != 0) {
    if (error) *error = RemoteStatusError(EnginePipeCommand::SyllableBounds, status);
    return false;
  }
  if (response.size() < 4) {
    if (error) *error = L"shared engine syllable response was truncated";
    return false;
  }

  const uint32_t count =
      static_cast<uint32_t>(response[0]) | (static_cast<uint32_t>(response[1]) << 8) |
      (static_cast<uint32_t>(response[2]) << 16) | (static_cast<uint32_t>(response[3]) << 24);
  if (response.size() < 4 + static_cast<size_t>(count) * sizeof(uint32_t)) {
    if (error) *error = L"shared engine syllable response payload was truncated";
    return false;
  }
  const size_t copyCount = (std::min)(cap, static_cast<size_t>(count));
  for (size_t i = 0; i < copyCount; ++i) {
    const size_t offset = 4 + i * sizeof(uint32_t);
    out[i] = static_cast<uint32_t>(response[offset]) | (static_cast<uint32_t>(response[offset + 1]) << 8) |
             (static_cast<uint32_t>(response[offset + 2]) << 16) |
             (static_cast<uint32_t>(response[offset + 3]) << 24);
  }
  if (written) *written = copyCount;
  return true;
}

void ResetBridgeLocked() {
  const uint32_t pendingModeFlags = g_bridge.pendingModeFlags;
  ClearLocalLookupCache();
  SetLocalLookupCacheSignature(std::wstring());
  CloseRemotePipeOnlyLocked();
  g_bridge = {};
  g_bridge.pendingModeFlags = pendingModeFlags;
  g_pendingModeFlagsMirror.store(pendingModeFlags, std::memory_order_release);
}

void EnsureEngineWatchdogStarted(const std::filesystem::path& moduleDir);
void EnsureRetryLoopScheduled();

bool EnsureLoadedLocked() {
  if (g_bridge.initialized) {
    if (g_bridge.backend == EngineBackend::Remote) return true;
  }

  const auto moduleDir = ModuleDir();
  if (moduleDir.empty()) {
    SetFailureDetailLocked(L"engine init: ModuleDir() returned empty path");
    return false;
  }
  AppendEngineStateLogDeduped(L"engine init: moduleDir=" + moduleDir.wstring());

  const bool helperStarted = EnsureEngineHelperRunning(moduleDir);
  EnsureEngineWatchdogStarted(moduleDir);
  std::wstring lexiconError;
  const auto lexiconDir = ResolveTrustedLexiconDir(moduleDir, &lexiconError);
  if (lexiconDir.empty()) {
    ResetBridgeLocked();
    const std::wstring detail = lexiconError.empty() ? L"trusted lexicon directory not found" : lexiconError;
    SetFailureDetailLocked(detail);
    AppendEngineFailureLogDeduped(L"engine init: lexicon resolution failed: " + detail +
                                  L" (moduleDir=" + moduleDir.wstring() + L")");
    return false;
  }
  AppendEngineStateLogDeduped(L"engine init: lexiconDir=" + lexiconDir.wstring());

  // The helper claims its single-instance mutex before its listener thread has
  // created the first named-pipe instance. Warmup runs off the keystroke path,
  // so absorb that short launch race here instead of failing a 500 ms health
  // handshake and making the first composition wait for the retry loop.
  if (!WaitForEnginePipeReady(moduleDir,
                              helperStarted ? kEnginePipeStartupReadyTimeoutMs
                                            : kEnginePipeHealthTimeoutMs)) {
    AppendEngineStateLogDeduped(L"engine prestart: pipe not ready before health handshake");
  }

  std::wstring remoteError;
  bool remoteVerified = false;
  for (int attempt = 0; attempt < 3 && !remoteVerified; ++attempt) {
    if (attempt > 0) {
      Sleep(static_cast<DWORD>(80 * attempt));
      EnsureEngineHelperRunning(moduleDir);
      (void)WaitForEnginePipeReady(moduleDir, kEnginePipeHealthTimeoutMs);
    }
    remoteError.clear();
    remoteVerified = VerifyRemoteInstanceLocked(moduleDir, &remoteError);
  }
  if (!remoteVerified) {
    ResetBridgeLocked();
    SetFailureDetailLocked(remoteError.empty() ? L"shared engine helper health check failed"
                                               : L"shared helper: " + remoteError);
    return false;
  }

  if (EnsureRemoteLoadedLocked(lexiconDir, &remoteError)) {
    ClearFailureDetailLocked();
    return true;
  }

  ResetBridgeLocked();
  SetFailureDetailLocked(remoteError.empty() ? L"shared engine helper unavailable"
                                             : L"shared helper: " + remoteError);
  return false;
}

void TouchEngineUseTime() {
  g_lastEngineUseTime.store(GetTickCount64(), std::memory_order_release);
}

void IdleKeepaliveCheckWorker() {
  while (g_idleWatcherRunning.load(std::memory_order_acquire)) {
    Sleep(kIdleCheckIntervalMs);
    if (!g_idleWatcherRunning.load(std::memory_order_acquire)) break;
    const SrfEngineState state = g_engineState.load(std::memory_order_acquire);
    if (state != SrfEngineState::Ready) continue;
    const ULONGLONG lastUse = g_lastEngineUseTime.load(std::memory_order_acquire);
    if (lastUse == 0) continue;
    if (GetTickCount64() - lastUse < kIdleKeepaliveTimeoutMs) continue;
    std::lock_guard<std::mutex> guard(g_mutex);
    const ULONGLONG lastUseCheck = g_lastEngineUseTime.load(std::memory_order_acquire);
    if (GetTickCount64() - lastUseCheck < kIdleKeepaliveTimeoutMs) continue;
    if (g_engineState.load(std::memory_order_acquire) != SrfEngineState::Ready) continue;
    if (EnsureLoadedLocked()) {
      TouchEngineUseTime();
      continue;
    }
    PublishEngineState(SrfEngineState::Failed);
    EnsureRetryLoopScheduled();
  }
}

void EnsureIdleWatcherStarted() {
  bool expected = false;
  if (g_idleWatcherRunning.compare_exchange_strong(expected, true, std::memory_order_acq_rel)) {
    if (!StartDetachedBackgroundWorker([] { IdleKeepaliveCheckWorker(); })) {
      g_idleWatcherRunning.store(false, std::memory_order_release);
    }
  }
}

void EngineWatchdogWorker(std::filesystem::path moduleDir) {
  while (g_trayWatchdogRunning.load(std::memory_order_acquire)) {
    Sleep(kEngineWatchdogIntervalMs);
    if (!g_trayWatchdogRunning.load(std::memory_order_acquire)) break;
    const std::wstring mutexName = EngineMutexNameForModuleDir(moduleDir);
    HANDLE mutex = OpenMutexW(SYNCHRONIZE, FALSE, mutexName.c_str());
    if (mutex) {
      CloseHandle(mutex);
      continue;
    }
    (void)EnsureEngineHelperRunning(moduleDir);
  }
}

void EnsureEngineWatchdogStarted(const std::filesystem::path& moduleDir) {
  bool expected = false;
  if (g_trayWatchdogRunning.compare_exchange_strong(expected, true, std::memory_order_acq_rel)) {
    if (!StartDetachedBackgroundWorker([moduleDir] { EngineWatchdogWorker(moduleDir); })) {
      g_trayWatchdogRunning.store(false, std::memory_order_release);
    }
  }
}

void ResetRetryLoopState() {
  g_retryLoopGeneration.fetch_add(1, std::memory_order_acq_rel);
  g_retryLoopInFlight.store(false, std::memory_order_release);
}

DWORD RetryBackoffMsForAttempt(size_t attempt) {
  const size_t maxIndex = kRetryBackoffMs.size() - 1;
  const size_t index = attempt < maxIndex ? attempt : maxIndex;
  return kRetryBackoffMs[index];
}

void WarmupEngineWorker();

void RetryLoopWorker(unsigned long long generation) {
  size_t attempt = 0;
  while (true) {
    if (generation != g_retryLoopGeneration.load(std::memory_order_acquire)) break;
    if (!g_retryOnFailureEnabled.load(std::memory_order_acquire)) break;
    if (g_engineState.load(std::memory_order_acquire) == SrfEngineState::Ready) break;

    Sleep(RetryBackoffMsForAttempt(attempt));

    if (generation != g_retryLoopGeneration.load(std::memory_order_acquire)) break;
    if (!g_retryOnFailureEnabled.load(std::memory_order_acquire)) break;
    if (g_engineState.load(std::memory_order_acquire) == SrfEngineState::Ready) break;

    if (g_warmupInFlight.exchange(true, std::memory_order_acq_rel)) continue;
    PublishEngineState(SrfEngineState::Loading);
    WarmupEngineWorker();
    if (g_engineState.load(std::memory_order_acquire) == SrfEngineState::Ready) break;
    ++attempt;
  }

  g_retryLoopInFlight.store(false, std::memory_order_release);
}

void EnsureRetryLoopScheduled() {
  if (!g_retryOnFailureEnabled.load(std::memory_order_acquire)) return;
  if (g_engineState.load(std::memory_order_acquire) == SrfEngineState::Ready) return;

  bool expected = false;
  if (!g_retryLoopInFlight.compare_exchange_strong(expected, true, std::memory_order_acq_rel)) {
    return;
  }

  const unsigned long long generation = g_retryLoopGeneration.load(std::memory_order_acquire);
  if (!StartDetachedBackgroundWorker([generation] { RetryLoopWorker(generation); })) {
    g_retryLoopInFlight.store(false, std::memory_order_release);
  }
}

bool ShouldStartUserTriggeredWarmup() {
  const ULONGLONG now = GetTickCount64();
  ULONGLONG last = g_lastUserTriggeredWarmupTick.load(std::memory_order_acquire);
  while (last == 0 || now < last || now - last >= kFailedStateWarmupCooldownMs) {
    if (g_lastUserTriggeredWarmupTick.compare_exchange_weak(last, now, std::memory_order_acq_rel,
                                                            std::memory_order_acquire)) {
      return true;
    }
  }
  return false;
}

bool StartWarmupWorkerAsync() {
  if (g_warmupInFlight.exchange(true, std::memory_order_acq_rel)) return false;

  PublishEngineState(SrfEngineState::Loading);
  if (StartDetachedBackgroundWorker([] { WarmupEngineWorker(); })) return true;

  g_warmupInFlight.store(false, std::memory_order_release);
  PublishEngineState(SrfEngineState::Failed);
  EnsureRetryLoopScheduled();
  return false;
}

void WarmupEngineWorker() {
  std::wstring failureSnap;
  const bool initialized = [&] {
    std::lock_guard<std::mutex> guard(g_mutex);
    const bool ok = EnsureLoadedLocked();
    if (!ok) failureSnap = g_lastEngineFailure;
    return ok;
  }();

  PublishEngineState(initialized ? SrfEngineState::Ready : SrfEngineState::Failed);
  g_warmupInFlight.store(false, std::memory_order_release);
  if (initialized) {
    TouchEngineUseTime();
    EnsureIdleWatcherStarted();
    g_lastUserTriggeredWarmupTick.store(0, std::memory_order_release);
    ResetRetryLoopState();
    SrfTip_PrewarmSingleLetterLookupCacheAsync();
  } else {
    EnsureRetryLoopScheduled();
  }
  if (!initialized) AppendEngineFailureLogDeduped(failureSnap);
}

bool EnsureReadyNow() {
  std::wstring failureSnap;
  const bool initialized = [&] {
    std::lock_guard<std::mutex> guard(g_mutex);
    const bool ok = EnsureLoadedLocked();
    if (!ok) failureSnap = g_lastEngineFailure;
    return ok;
  }();

  PublishEngineState(initialized ? SrfEngineState::Ready : SrfEngineState::Failed);
  g_warmupInFlight.store(false, std::memory_order_release);
  if (initialized) {
    TouchEngineUseTime();
    EnsureIdleWatcherStarted();
    ResetRetryLoopState();
  } else {
    EnsureRetryLoopScheduled();
  }
  if (!initialized) AppendEngineFailureLogDeduped(failureSnap);
  return initialized;
}

}  // namespace

unsigned long long SrfTip_NextLookupRequestId() {
  LARGE_INTEGER counter = {};
  unsigned long long candidate = 0;
  if (QueryPerformanceCounter(&counter) && counter.QuadPart > 0) {
    candidate = static_cast<unsigned long long>(counter.QuadPart);
  } else {
    candidate = static_cast<unsigned long long>(GetTickCount64());
  }

  unsigned long long current = g_latestLookupRequestId.load(std::memory_order_acquire);
  for (;;) {
    const unsigned long long next = (std::max)(candidate, current + 1);
    if (g_latestLookupRequestId.compare_exchange_weak(current, next, std::memory_order_acq_rel,
                                                       std::memory_order_acquire)) {
      return next;
    }
  }
}

void SrfTip_CancelPendingLookupBefore(unsigned long long requestId) {
  if (requestId == 0) return;
  unsigned long long current =
      g_latestLookupCancelRequestId.load(std::memory_order_acquire);
  while (requestId > current &&
         !g_latestLookupCancelRequestId.compare_exchange_weak(
             current, requestId, std::memory_order_acq_rel, std::memory_order_acquire)) {
  }
  if (requestId < current) return;
  if (g_lookupCancelWorkerRunning.exchange(true, std::memory_order_acq_rel)) return;
  if (!StartDetachedBackgroundWorker([] { LookupCancelWorkerMain(); })) {
    g_lookupCancelWorkerRunning.store(false, std::memory_order_release);
  }
}

void SrfTip_ClearLookupCache() {
  ClearLocalLookupCache();
}

bool SrfTip_InitializeEngine() {
  const SrfEngineState state = g_engineState.load(std::memory_order_acquire);
  if (state == SrfEngineState::Ready) return true;
  SrfTip_WarmupEngineAsync();
  return false;
}

void SrfTip_SetRetryOnFailureEnabled(bool enabled) {
  const bool previous =
      g_retryOnFailureEnabled.exchange(enabled, std::memory_order_acq_rel);
  if (previous != enabled) {
    ResetRetryLoopState();
  }
  if (enabled && g_engineState.load(std::memory_order_acquire) == SrfEngineState::Failed) {
    EnsureRetryLoopScheduled();
  }
}

void SrfTip_WarmupEngineAsync() {
  const SrfEngineState state = g_engineState.load(std::memory_order_acquire);
  if (state == SrfEngineState::Ready) return;
  if (state == SrfEngineState::Failed) {
    if (ShouldStartUserTriggeredWarmup()) {
      if (StartWarmupWorkerAsync()) return;
    }
    EnsureRetryLoopScheduled();
    return;
  }
  (void)StartWarmupWorkerAsync();
}

SrfEngineState SrfTip_GetEngineState() {
  return g_engineState.load(std::memory_order_acquire);
}

std::wstring SrfTip_GetEngineFailureDetail() {
  std::lock_guard<std::mutex> guard(g_mutex);
  return g_lastEngineFailure;
}

size_t SrfTip_SyllableBoundaryOffsetsUtf16(const wchar_t* text, size_t unitCount, uint32_t* out,
                                           size_t cap) {
  if (!text || !out || cap == 0 || unitCount > kMaxBridgeInputUnits) return 0;
  TouchEngineUseTime();
  const SrfEngineState state = g_engineState.load(std::memory_order_acquire);
  if (state != SrfEngineState::Ready) {
    SrfTip_WarmupEngineAsync();
    return 0;
  }
  std::wstring failureSnap;
  std::unique_lock<std::mutex> guard(g_mutex, std::try_to_lock);
  if (!guard.owns_lock()) return 0;
  if (!EnsureLoadedLocked()) return 0;
  if (g_bridge.backend == EngineBackend::Remote) {
    size_t written = 0;
    std::wstring error;
    if (RemoteSyllableBoundsLocked(text, unitCount, out, cap, &written, &error)) return written;
    SetFailureDetailLocked(error.empty() ? L"shared engine syllable bounds failed" : error);
    failureSnap = g_lastEngineFailure;
    InvalidateRemoteBackendLocked();
    PublishEngineState(SrfEngineState::Failed);
    EnsureRetryLoopScheduled();
  }
  if (!failureSnap.empty()) AppendEngineFailureLogDeduped(failureSnap);
  return 0;
}

bool SrfTip_TryGetCachedLookupCandidates(const std::wstring& reading,
                                         std::vector<std::wstring>& candidates,
                                         std::vector<std::wstring>* metaScores) {
  candidates.clear();
  if (metaScores) metaScores->clear();
  if (reading.empty() || reading.size() > kMaxBridgeInputUnits) return false;
  const uint32_t modeFlags = g_pendingModeFlagsMirror.load(std::memory_order_acquire);
  return TryGetLocalLookupCache(reading, modeFlags, candidates, metaScores);
}

void SrfTip_SetEngineModeFlags(unsigned long flags) {
  g_pendingModeFlagsMirror.store(static_cast<uint32_t>(flags), std::memory_order_release);
  std::unique_lock<std::mutex> guard(g_mutex, std::try_to_lock);
  if (guard.owns_lock()) {
    g_bridge.pendingModeFlags = static_cast<uint32_t>(flags);
  }
  if (g_engineState.load(std::memory_order_acquire) == SrfEngineState::Ready) {
    SrfTip_PrewarmSingleLetterLookupCacheAsync();
  }
}

bool ProcessQueuedLearnRequest(PendingLearnRequest request) {
  TouchEngineUseTime();
  unsigned int busyAttempt = 0;
  for (;;) {
    const ULONGLONG deadline = GetTickCount64() + kAsyncLearnReadyWaitMs;
    SrfEngineState state = g_engineState.load(std::memory_order_acquire);
    while (state != SrfEngineState::Ready && GetTickCount64() < deadline) {
      SrfTip_WarmupEngineAsync();
      Sleep(10);
      state = g_engineState.load(std::memory_order_acquire);
    }
    if (state != SrfEngineState::Ready) {
      SrfTip_WarmupEngineAsync();
      return false;
    }

    std::wstring failureSnap;
    std::wstring error;
    bool completed = false;
    bool engineBusy = false;
    bool madeProgress = false;
    {
      std::unique_lock<std::mutex> guard(g_mutex);
      if (!EnsureLoadedLocked()) return false;
      if (g_bridge.backend == EngineBackend::Remote) {
        switch (request.kind) {
          case PendingLearnKind::Commit:
            if (RemoteLearnLocked(request.reading, request.committedText, request.flags, &error)) {
              // The lookup cache may have been filled again while the async
              // learn request was waiting in the queue.  Pruning only removes
              // non-hot entries, so explicitly invalidate this reading to
              // make the next lookup observe the updated user lexicon.
              InvalidateLocalLookupCacheReading(request.reading);
              if (request.repeatCount > 0) --request.repeatCount;
              completed = request.repeatCount == 0;
              madeProgress = true;
            }
            break;
          case PendingLearnKind::Correction:
            completed = RemoteLearnCorrectionLocked(request.reading, request.correctedReading,
                                                     request.committedText, &error);
            break;
          case PendingLearnKind::SelectionFeedback:
            completed = RemoteLearnSelectionFeedbackLocked(
                request.reading, request.committedText, request.selectedIndex, request.page,
                request.skippedCandidates, &error);
            break;
        }

        if (!completed && !error.empty()) engineBusy = IsRemoteEngineBusyError(error);
        if (!completed && !engineBusy) {
          SetFailureDetailLocked(error.empty() ? L"shared engine learning request failed" : error);
          failureSnap = g_lastEngineFailure;
          InvalidateRemoteBackendLocked();
          PublishEngineState(SrfEngineState::Failed);
          EnsureRetryLoopScheduled();
        }
      } else {
        failureSnap = L"shared engine backend is not connected";
        PublishEngineState(SrfEngineState::Failed);
        EnsureRetryLoopScheduled();
      }
    }

    if (completed) {
      PruneLocalLookupCachePreservingHot();
      return true;
    }
    if (madeProgress) {
      busyAttempt = 0;
      continue;
    }
    if (engineBusy) {
      ++busyAttempt;
      const DWORD backoffMs = (std::min)(10u + busyAttempt * 10u, 200u);
      Sleep(backoffMs);
      continue;
    }
    if (failureSnap.empty()) failureSnap = L"shared engine learning request failed";
    AppendEngineFailureLogDeduped(failureSnap);
    return false;
  }
}

void PostLearnCompletion(const PendingLearnRequest& request, bool succeeded) {
  if (!request.completionWindow || request.completionMessage == 0 || request.completionId == 0) {
    return;
  }
  (void)PostMessageW(request.completionWindow, request.completionMessage,
                     static_cast<WPARAM>(request.completionId), succeeded ? 1 : 0);
}

void LearnCommitWorkerMain() {
  for (;;) {
    PendingLearnRequest request;
    {
      std::lock_guard<std::mutex> guard(g_learnQueueMutex);
      if (g_learnQueue.empty()) {
        g_learnWorkerRunning.store(false, std::memory_order_release);
        return;
      }
      request = std::move(g_learnQueue.front());
      g_learnQueue.pop_front();
    }
    const bool completed = ProcessQueuedLearnRequest(request);
    PostLearnCompletion(request, completed);
  }
}

void EnsureLearnCommitWorkerRunning() {
  bool expected = false;
  if (!g_learnWorkerRunning.compare_exchange_strong(expected, true,
                                                    std::memory_order_acq_rel)) {
    return;
  }
  if (!StartDetachedBackgroundWorker([] { LearnCommitWorkerMain(); })) {
    g_learnWorkerRunning.store(false, std::memory_order_release);
  }
}

int LearnRequestPriority(const PendingLearnRequest& request) {
  if (request.kind == PendingLearnKind::Correction) return 3;
  if (request.kind == PendingLearnKind::SelectionFeedback) return 1;
  if ((request.flags & kSrfLearnCommitComposedPhrase) != 0) return 2;
  if ((request.flags & kSrfLearnCommitWeak) != 0) return 0;
  return 1;
}

void DropLowestPriorityLearnRequestLocked() {
  if (g_learnQueue.empty()) return;
  auto dropIt = g_learnQueue.begin();
  int dropPriority = LearnRequestPriority(*dropIt);
  for (auto it = std::next(g_learnQueue.begin()); it != g_learnQueue.end(); ++it) {
    const int priority = LearnRequestPriority(*it);
    if (priority < dropPriority) {
      dropPriority = priority;
      dropIt = it;
      if (dropPriority == 0) break;
    }
  }
  PostLearnCompletion(*dropIt, false);
  g_learnQueue.erase(dropIt);
}

unsigned long long QueueLearnCommitAsync(const std::wstring& reading,
                                         const std::wstring& committedText,
                                         unsigned long flags, HWND completionWindow = nullptr,
                                         UINT completionMessage = 0) {
  const unsigned long long completionId =
      completionWindow && completionMessage != 0
          ? g_nextLearnCompletionId.fetch_add(1, std::memory_order_relaxed)
          : 0;
  {
    std::lock_guard<std::mutex> guard(g_learnQueueMutex);
    for (auto it = g_learnQueue.begin(); it != g_learnQueue.end(); ++it) {
      if (completionId == 0 && it->completionId == 0 &&
          it->kind == PendingLearnKind::Commit && it->flags == flags &&
          it->reading == reading && it->committedText == committedText) {
        it->repeatCount = (std::min)(it->repeatCount + 1, kLearnQueueMaxRepeatCount);
        PendingLearnRequest merged = std::move(*it);
        g_learnQueue.erase(it);
        g_learnQueue.push_back(std::move(merged));
        break;
      }
    }
    if (completionId != 0 || g_learnQueue.empty() || g_learnQueue.back().completionId != 0 ||
        g_learnQueue.back().kind != PendingLearnKind::Commit ||
        g_learnQueue.back().reading != reading ||
        g_learnQueue.back().committedText != committedText || g_learnQueue.back().flags != flags) {
      while (g_learnQueue.size() >= kLearnQueueCapacity) {
        DropLowestPriorityLearnRequestLocked();
      }
      PendingLearnRequest request;
      request.kind = PendingLearnKind::Commit;
      request.reading = reading;
      request.committedText = committedText;
      request.flags = flags;
      request.repeatCount = 1;
      request.completionWindow = completionWindow;
      request.completionMessage = completionMessage;
      request.completionId = completionId;
      g_learnQueue.push_back(std::move(request));
    }
  }
  EnsureLearnCommitWorkerRunning();
  return completionId;
}

void QueueLearnCorrectionAsync(const std::wstring& rawReading,
                               const std::wstring& correctedReading,
                               const std::wstring& committedText) {
  {
    std::lock_guard<std::mutex> guard(g_learnQueueMutex);
    while (g_learnQueue.size() >= kLearnQueueCapacity) {
      DropLowestPriorityLearnRequestLocked();
    }
    PendingLearnRequest request;
    request.kind = PendingLearnKind::Correction;
    request.reading = rawReading;
    request.correctedReading = correctedReading;
    request.committedText = committedText;
    g_learnQueue.push_back(std::move(request));
  }
  EnsureLearnCommitWorkerRunning();
}

void QueueLearnSelectionFeedbackAsync(const std::wstring& reading,
                                      const std::wstring& committedText,
                                      unsigned long selectedIndex,
                                      unsigned long page,
                                      std::vector<std::wstring> skippedCandidates) {
  {
    std::lock_guard<std::mutex> guard(g_learnQueueMutex);
    while (g_learnQueue.size() >= kLearnQueueCapacity) {
      DropLowestPriorityLearnRequestLocked();
    }
    PendingLearnRequest request;
    request.kind = PendingLearnKind::SelectionFeedback;
    request.reading = reading;
    request.committedText = committedText;
    request.selectedIndex = selectedIndex;
    request.page = page;
    request.skippedCandidates = std::move(skippedCandidates);
    g_learnQueue.push_back(std::move(request));
  }
  EnsureLearnCommitWorkerRunning();
}

void SrfTip_LearnCommit(const std::wstring& reading, const std::wstring& committedText) {
  SrfTip_LearnCommitEx(reading, committedText, kSrfLearnCommitDefault);
}

void SrfTip_LearnCommitEx(const std::wstring& reading, const std::wstring& committedText,
                          unsigned long flags) {
  if (reading.empty() || committedText.empty()) return;
  if (reading.size() > kMaxBridgeInputUnits || committedText.size() > kMaxLearnPhraseUnits) return;
  if ((g_pendingModeFlagsMirror.load(std::memory_order_acquire) & kRustModeTraditionalOutput) != 0) {
    return;
  }
  PruneLocalLookupCachePreservingHot();
  TouchEngineUseTime();
  QueueLearnCommitAsync(reading, committedText, flags);
}

unsigned long long SrfTip_LearnCommitExWithCompletion(
    const std::wstring& reading, const std::wstring& committedText, unsigned long flags,
    HWND completionWindow, UINT completionMessage) {
  if (reading.empty() || committedText.empty() || !completionWindow || completionMessage == 0) {
    return 0;
  }
  if (reading.size() > kMaxBridgeInputUnits || committedText.size() > kMaxLearnPhraseUnits) {
    return 0;
  }
  if ((g_pendingModeFlagsMirror.load(std::memory_order_acquire) & kRustModeTraditionalOutput) != 0) {
    return 0;
  }
  PruneLocalLookupCachePreservingHot();
  TouchEngineUseTime();
  return QueueLearnCommitAsync(reading, committedText, flags, completionWindow,
                               completionMessage);
}

void SrfTip_LearnCorrection(const std::wstring& rawReading,
                            const std::wstring& correctedReading,
                            const std::wstring& committedText) {
  if (rawReading.empty() || correctedReading.empty() || committedText.empty()) return;
  if (rawReading.size() > kMaxBridgeInputUnits || correctedReading.size() > kMaxBridgeInputUnits ||
      committedText.size() > kMaxLearnPhraseUnits) {
    return;
  }
  if ((g_pendingModeFlagsMirror.load(std::memory_order_acquire) & kRustModeTraditionalOutput) != 0) {
    return;
  }
  PruneLocalLookupCachePreservingHot();
  TouchEngineUseTime();
  QueueLearnCorrectionAsync(rawReading, correctedReading, committedText);
}

void SrfTip_LearnSelectionFeedback(const std::wstring& reading,
                                   const std::wstring& committedText,
                                   unsigned long selectedIndex,
                                   unsigned long page,
                                   const std::vector<std::wstring>& skippedCandidates) {
  if (reading.empty() || committedText.empty()) return;
  if (reading.size() > kMaxBridgeInputUnits || committedText.size() > kMaxLearnPhraseUnits) {
    return;
  }
  if ((g_pendingModeFlagsMirror.load(std::memory_order_acquire) & kRustModeTraditionalOutput) != 0) {
    return;
  }
  PruneLocalLookupCachePreservingHot();
  TouchEngineUseTime();
  std::vector<std::wstring> queuedSkipped;
  queuedSkipped.reserve(skippedCandidates.size() < kMaxSelectionFeedbackSkipped
                            ? skippedCandidates.size()
                            : kMaxSelectionFeedbackSkipped);
  for (const auto& candidate : skippedCandidates) {
    if (candidate.empty() || candidate == committedText || candidate.size() > kMaxLearnPhraseUnits) {
      continue;
    }
    queuedSkipped.push_back(candidate);
    if (queuedSkipped.size() >= kMaxSelectionFeedbackSkipped) break;
  }
  QueueLearnSelectionFeedbackAsync(reading, committedText, selectedIndex, page,
                                   std::move(queuedSkipped));
}

void SrfTip_ResetLearningContext() {
  if ((g_pendingModeFlagsMirror.load(std::memory_order_acquire) & kRustModeTraditionalOutput) != 0) {
    return;
  }
  PruneLocalLookupCachePreservingHot();

  const SrfEngineState state = g_engineState.load(std::memory_order_acquire);
  if (state != SrfEngineState::Ready) {
    return;
  }

  std::wstring failureSnap;
  std::unique_lock<std::mutex> guard(g_mutex, std::try_to_lock);
  if (!guard.owns_lock()) return;
  if (!EnsureLoadedLocked()) return;
  if (g_bridge.backend == EngineBackend::Remote) {
    std::wstring error;
    if (RemoteResetLearningContextLocked(&error)) {
      guard.unlock();
      PruneLocalLookupCachePreservingHot();
      return;
    }
    SetFailureDetailLocked(error.empty() ? L"shared engine reset learning context failed" : error);
    failureSnap = g_lastEngineFailure;
    InvalidateRemoteBackendLocked();
    PublishEngineState(SrfEngineState::Failed);
    EnsureRetryLoopScheduled();
  }
  if (!failureSnap.empty()) AppendEngineFailureLogDeduped(failureSnap);
}

bool SrfTip_SetCandidatePin(const std::wstring& reading, const std::wstring& committedText,
                          bool pinned) {
  if (reading.empty() || committedText.empty()) return false;
  if (reading.size() > kMaxBridgeInputUnits || committedText.size() > kMaxLearnPhraseUnits) return false;
  if ((g_pendingModeFlagsMirror.load(std::memory_order_acquire) & kRustModeTraditionalOutput) != 0) {
    return false;
  }
  InvalidateLocalLookupCacheReading(reading);
  TouchEngineUseTime();

  const SrfEngineState state = g_engineState.load(std::memory_order_acquire);
  if (state != SrfEngineState::Ready) {
    SrfTip_WarmupEngineAsync();
    return false;
  }

  std::wstring failureSnap;
  std::unique_lock<std::mutex> guard(g_mutex, std::try_to_lock);
  if (!guard.owns_lock()) return false;
  if (!EnsureLoadedLocked()) return false;
  if (g_bridge.backend == EngineBackend::Remote) {
    std::wstring error;
    if (RemoteSetCandidatePinLocked(reading, committedText, pinned, &error)) return true;
    SetFailureDetailLocked(error.empty() ? L"shared engine candidate pin failed" : error);
    failureSnap = g_lastEngineFailure;
    InvalidateRemoteBackendLocked();
    PublishEngineState(SrfEngineState::Failed);
    EnsureRetryLoopScheduled();
  }
  if (!failureSnap.empty()) AppendEngineFailureLogDeduped(failureSnap);
  return false;
}

void SrfTip_ApplyCandidateAction(const std::wstring& reading,
                                 const std::wstring& committedText,
                                 SrfCandidateAction action) {
  if (reading.empty() || committedText.empty()) return;
  if (reading.size() > kMaxBridgeInputUnits || committedText.size() > kMaxLearnPhraseUnits) return;
  if ((g_pendingModeFlagsMirror.load(std::memory_order_acquire) & kRustModeTraditionalOutput) != 0) {
    return;
  }
  InvalidateLocalLookupCacheReading(reading);
  TouchEngineUseTime();

  const SrfEngineState state = g_engineState.load(std::memory_order_acquire);
  if (state != SrfEngineState::Ready) {
    SrfTip_WarmupEngineAsync();
    return;
  }

  std::wstring failureSnap;
  std::unique_lock<std::mutex> guard(g_mutex, std::try_to_lock);
  if (!guard.owns_lock()) return;
  if (!EnsureLoadedLocked()) return;
  if (g_bridge.backend == EngineBackend::Remote) {
    std::wstring error;
    if (RemoteCandidateActionLocked(reading, committedText, action, &error)) {
      guard.unlock();
      InvalidateLocalLookupCacheReading(reading);
      return;
    }
    SetFailureDetailLocked(error.empty() ? L"shared engine candidate action failed" : error);
    failureSnap = g_lastEngineFailure;
    InvalidateRemoteBackendLocked();
    PublishEngineState(SrfEngineState::Failed);
    EnsureRetryLoopScheduled();
  }
  if (!failureSnap.empty()) AppendEngineFailureLogDeduped(failureSnap);
}

void SrfTip_RecordClipboardText(const std::wstring& text) {
  if (text.empty() || text.size() > kMaxClipboardTextUnits) return;
  TouchEngineUseTime();

  const SrfEngineState state = g_engineState.load(std::memory_order_acquire);
  if (state != SrfEngineState::Ready) {
    SrfTip_WarmupEngineAsync();
    return;
  }

  std::wstring failureSnap;
  std::unique_lock<std::mutex> guard(g_mutex);
  if (!EnsureLoadedLocked()) return;
  if (g_bridge.backend == EngineBackend::Remote) {
    std::wstring error;
    if (RemoteRecordClipboardLocked(text, &error)) return;
    SetFailureDetailLocked(error.empty() ? L"shared engine clipboard record failed" : error);
    failureSnap = g_lastEngineFailure;
    InvalidateRemoteBackendLocked();
    PublishEngineState(SrfEngineState::Failed);
    EnsureRetryLoopScheduled();
  }
  if (!failureSnap.empty()) AppendEngineFailureLogDeduped(failureSnap);
}

bool SrfTip_ResolveClipboardText(const std::wstring& id, std::wstring* text) {
  if (text) text->clear();
  if (id.empty() || !text) return false;
  TouchEngineUseTime();

  const SrfEngineState state = g_engineState.load(std::memory_order_acquire);
  if (state != SrfEngineState::Ready) {
    SrfTip_WarmupEngineAsync();
    return false;
  }

  std::wstring failureSnap;
  std::unique_lock<std::mutex> guard(g_mutex);
  if (!EnsureLoadedLocked()) return false;
  if (g_bridge.backend == EngineBackend::Remote) {
    std::wstring error;
    if (RemoteResolveClipboardLocked(id, text, &error)) return true;
    SetFailureDetailLocked(error.empty() ? L"shared engine clipboard resolve failed" : error);
    failureSnap = g_lastEngineFailure;
    InvalidateRemoteBackendLocked();
    PublishEngineState(SrfEngineState::Failed);
    EnsureRetryLoopScheduled();
  }
  if (!failureSnap.empty()) AppendEngineFailureLogDeduped(failureSnap);
  return false;
}

bool TryServeLookupFromStaleCache(const std::wstring& reading, uint32_t modeFlags,
                                  std::vector<std::wstring>& candidates,
                                  std::vector<std::wstring>* metaScores,
                                  unsigned long long requestId,
                                  const wchar_t* reason) {
  if (!TryGetStaleLocalLookupCache(reading, modeFlags, candidates, metaScores)) return false;
  std::wstring detail = L"lookup fallback=local_cache stale=1 reason=";
  detail += reason ? reason : L"unknown";
  detail += L" candidates=";
  detail += std::to_wstring(candidates.size());
  if (requestId != 0) {
    detail += L" request_id=";
    detail += std::to_wstring(requestId);
  }
  AppendEngineFailureLogDeduped(detail);
  return true;
}

SrfLookupCandidatesStatus SrfTip_LookupCandidates(const std::wstring& reading,
                                                  std::vector<std::wstring>& candidates,
                                                  std::vector<std::wstring>* metaScores,
                                                  unsigned long long requestId) {
  candidates.clear();
  if (metaScores) metaScores->clear();
  if (reading.empty() || reading.size() > kMaxBridgeInputUnits) {
    return SrfLookupCandidatesStatus::Empty;
  }
  if (requestId == 0) requestId = SrfTip_NextLookupRequestId();
  TouchEngineUseTime();
  PublishLatestLookupRequestId(requestId);
  const uint32_t modeFlags = g_pendingModeFlagsMirror.load(std::memory_order_acquire);
  if (TryGetLocalLookupCache(reading, modeFlags, candidates, metaScores)) {
    return SrfLookupCandidatesStatus::Ok;
  }

  const bool singleLetterLookup = SingleLetterIndex(reading) >= 0;
  const ULONGLONG singleLetterDeadline = GetTickCount64() + kSingleLetterReadyWaitMs;
  SrfEngineState state = g_engineState.load(std::memory_order_acquire);
  while (singleLetterLookup && state != SrfEngineState::Ready &&
         GetTickCount64() < singleLetterDeadline) {
    Sleep(0);
    state = g_engineState.load(std::memory_order_acquire);
  }
  if (state != SrfEngineState::Ready) {
    // Lookup never blocks the key path. If the engine is idle we kick off
    // warmup, and if it has failed we only schedule background retry.
    SrfTip_WarmupEngineAsync();
    if (TryServeLookupFromStaleCache(reading, modeFlags, candidates, metaScores, requestId,
                                     L"engine_not_ready")) {
      return SrfLookupCandidatesStatus::EngineNotReady;
    }
    return SrfLookupCandidatesStatus::EngineNotReady;
  }

  std::wstring failureSnap;
  std::unique_lock<std::mutex> guard(g_mutex, std::defer_lock);
  const ULONGLONG lockDeadline =
      GetTickCount64() + (singleLetterLookup ? kSingleLetterReadyWaitMs : kLookupBusyWaitMs);
  while (!guard.try_lock() && GetTickCount64() < lockDeadline) {
    if (IsLookupRequestSuperseded(requestId)) {
      TryServeLookupFromStaleCache(reading, modeFlags, candidates, metaScores, requestId,
                                   L"superseded");
      return SrfLookupCandidatesStatus::Superseded;
    }
    Sleep(0);
  }
  if (!guard.owns_lock()) {
    if (TryServeLookupFromStaleCache(reading, modeFlags, candidates, metaScores, requestId,
                                     L"bridge_lock_busy")) {
      return SrfLookupCandidatesStatus::BridgeBusy;
    }
    return SrfLookupCandidatesStatus::BridgeBusy;
  }
  if (!EnsureLoadedLocked()) {
    if (TryServeLookupFromStaleCache(reading, modeFlags, candidates, metaScores, requestId,
                                     L"ensure_loaded_failed")) {
      return SrfLookupCandidatesStatus::EnsureFailed;
    }
    return SrfLookupCandidatesStatus::EnsureFailed;
  }
  g_bridge.pendingModeFlags = modeFlags;

  int count = 0;
  if (g_bridge.backend == EngineBackend::Remote) {
    std::wstring error;
    if (!RemoteLookupLocked(reading, requestId, &candidates, metaScores, &count, &error)) {
      failureSnap = error.empty() ? L"shared engine lookup failed" : error;
      if (requestId != 0) {
        failureSnap = L"request_id=" + std::to_wstring(requestId) + L" " + failureSnap;
      }
      if (IsLookupSupersededError(failureSnap)) {
        ResetLookupTimeoutStreak();
        if (TryServeLookupFromStaleCache(reading, modeFlags, candidates, metaScores, requestId,
                                         L"superseded")) {
          return SrfLookupCandidatesStatus::Superseded;
        }
        return SrfLookupCandidatesStatus::Superseded;
      }
      const bool remoteBusy = IsRemoteEngineBusyError(failureSnap);
      if (!remoteBusy) InvalidateRemoteBackendLocked();
      if (remoteBusy) {
        ResetLookupTimeoutStreak();
        AppendEngineFailureLogDeduped(failureSnap);
        if (TryServeLookupFromStaleCache(reading, modeFlags, candidates, metaScores, requestId,
                                         L"remote_busy")) {
          return SrfLookupCandidatesStatus::RemoteBusy;
        }
        return SrfLookupCandidatesStatus::RemoteBusy;
      }
      if (IsLookupTimeoutError(failureSnap) && ShouldRestartHelperAfterLookupTimeout()) {
        std::wstring restartDetail = L"shared engine helper restart requested after ";
        restartDetail += std::to_wstring(kLookupTimeoutRestartThreshold);
        restartDetail += L" consecutive lookup timeouts; last=";
        restartDetail += failureSnap;
        SetFailureDetailLocked(restartDetail);
        WriteEngineRecoveryState(restartDetail);
        std::wstring shutdownError;
        if (!RequestRemoteHelperShutdownLocked(&shutdownError) && !shutdownError.empty()) {
          restartDetail += L"; shutdown=";
          restartDetail += shutdownError;
          SetFailureDetailLocked(restartDetail);
          WriteEngineRecoveryState(restartDetail);
        }
        PublishEngineState(SrfEngineState::Loading);
        guard.unlock();
        SrfTip_WarmupEngineAsync();
        AppendEngineFailureLogDeduped(restartDetail);
        if (TryServeLookupFromStaleCache(reading, modeFlags, candidates, metaScores, requestId,
                                         L"lookup_timeout_restart")) {
          return SrfLookupCandidatesStatus::TransientFailure;
        }
        return SrfLookupCandidatesStatus::TransientFailure;
      }
      if (IsTransientRemoteLookupError(failureSnap)) {
        if (!IsLookupTimeoutError(failureSnap)) ResetLookupTimeoutStreak();
        PublishEngineState(SrfEngineState::Loading);
        guard.unlock();
        SrfTip_WarmupEngineAsync();
        AppendEngineFailureLogDeduped(failureSnap);
        if (TryServeLookupFromStaleCache(reading, modeFlags, candidates, metaScores, requestId,
                                         L"transient_remote_error")) {
          return SrfLookupCandidatesStatus::TransientFailure;
        }
        return SrfLookupCandidatesStatus::TransientFailure;
      }
      ResetLookupTimeoutStreak();
      SetFailureDetailLocked(failureSnap);
      PublishEngineState(SrfEngineState::Failed);
      EnsureRetryLoopScheduled();
      if (TryServeLookupFromStaleCache(reading, modeFlags, candidates, metaScores, requestId,
                                       L"lookup_failed")) {
        return SrfLookupCandidatesStatus::Failed;
      }
      return SrfLookupCandidatesStatus::Failed;
    } else {
      ResetLookupTimeoutStreak();
      goto finish_lookup;
    }
  } else {
    ResetLookupTimeoutStreak();
    failureSnap = L"shared engine backend is not connected";
    PublishEngineState(SrfEngineState::Failed);
    EnsureRetryLoopScheduled();
    if (TryServeLookupFromStaleCache(reading, modeFlags, candidates, metaScores, requestId,
                                     L"backend_not_connected")) {
      return SrfLookupCandidatesStatus::BackendNotConnected;
    }
    return SrfLookupCandidatesStatus::BackendNotConnected;
  }
finish_lookup:
  if (!failureSnap.empty()) {
    AppendEngineFailureLogDeduped(failureSnap);
    return SrfLookupCandidatesStatus::Failed;
  }
  if (count <= 0 || candidates.empty()) return SrfLookupCandidatesStatus::Empty;
  PutLocalLookupCache(reading, modeFlags, candidates, metaScores);
  return SrfLookupCandidatesStatus::Ok;
}

void SrfTip_PrewarmSingleLetterLookupCacheAsync() {
  bool expected = false;
  if (!g_singleLetterPrewarmInFlight.compare_exchange_strong(expected, true,
                                                             std::memory_order_acq_rel)) {
    return;
  }

  if (!StartDetachedBackgroundWorker([] {
        struct PrewarmFlagGuard {
          ~PrewarmFlagGuard() {
            g_singleLetterPrewarmInFlight.store(false, std::memory_order_release);
          }
        } guard;

        const ULONGLONG deadline = GetTickCount64() + 2500;
        while (g_engineState.load(std::memory_order_acquire) != SrfEngineState::Ready &&
               GetTickCount64() < deadline) {
          SrfTip_WarmupEngineAsync();
          Sleep(25);
        }
        if (g_engineState.load(std::memory_order_acquire) != SrfEngineState::Ready) return;

        const uint32_t modeFlags = g_pendingModeFlagsMirror.load(std::memory_order_acquire);
        auto interactiveLookupQuiet = []() {
          const ULONGLONG last =
              g_lastInteractiveLookupTick.load(std::memory_order_acquire);
          if (last == 0) return true;
          const ULONGLONG now = GetTickCount64();
          return now < last || now - last >= kPrewarmQuietAfterInteractiveLookupMs;
        };
        auto prewarmReading = [&](const std::wstring& reading) {
          if (g_engineState.load(std::memory_order_acquire) != SrfEngineState::Ready) return false;
          if (g_pendingModeFlagsMirror.load(std::memory_order_acquire) != modeFlags) return false;

          std::vector<std::wstring> candidates;
          std::vector<std::wstring> meta;
          if (TryGetLocalLookupCache(reading, modeFlags, candidates, &meta)) return true;
          if (!interactiveLookupQuiet()) return false;

          std::unique_lock<std::mutex> bridgeGuard(g_mutex, std::try_to_lock);
          if (!bridgeGuard.owns_lock()) return false;
          if (!interactiveLookupQuiet()) return false;
          if (!g_bridge.initialized || g_bridge.backend != EngineBackend::Remote) return false;

          g_bridge.pendingModeFlags = modeFlags;
          int count = 0;
          std::wstring error;
          if (!RemoteLookupLocked(reading, 0, &candidates, &meta, &count, &error) || count <= 0 ||
              candidates.empty()) {
            return false;
          }
          PutLocalLookupCache(reading, modeFlags, candidates, &meta);
          Sleep(0);
          return true;
        };
        for (wchar_t ch = L'a'; ch <= L'z'; ++ch) {
          std::wstring reading(1, ch);
          if (!prewarmReading(reading)) return;
        }
        for (const auto& reading : HotLookupPrewarmReadings()) {
          if (!prewarmReading(reading)) return;
        }
      })) {
    g_singleLetterPrewarmInFlight.store(false, std::memory_order_release);
  }
}
