extern LONG g_cSrfTipObjects;
extern "C" void SrfTip_BackgroundWorkerAddRef();
extern "C" void SrfTip_BackgroundWorkerRelease();

namespace {

constexpr ULONGLONG kTsfTraceLogMaxBytes = 1024ULL * 1024ULL;
constexpr int kTsfTraceLogRotateKeep = 5;

// Runtime circuit breaker for games that do not tolerate Unicode injection.
// It is intentionally process-local: a new game build gets a fresh probe and
// no user configuration is rewritten behind their back.
std::mutex g_unicodeFallbackAppsMutex;
std::unordered_set<std::wstring> g_unicodeFallbackApps;

std::wstring UnicodeFallbackAppKey(const std::wstring& appName) {
  std::wstring key = appName;
  for (wchar_t& ch : key) {
    if (ch >= L'A' && ch <= L'Z') ch = static_cast<wchar_t>(ch - L'A' + L'a');
  }
  return key;
}

bool IsUnicodeFallbackApp(const std::wstring& appName) {
  if (appName.empty()) return false;
  const std::wstring key = UnicodeFallbackAppKey(appName);
  std::lock_guard<std::mutex> lock(g_unicodeFallbackAppsMutex);
  return g_unicodeFallbackApps.find(key) != g_unicodeFallbackApps.end();
}

void MarkUnicodeFallbackApp(const std::wstring& appName) {
  if (appName.empty()) return;
  const std::wstring key = UnicodeFallbackAppKey(appName);
  std::lock_guard<std::mutex> lock(g_unicodeFallbackAppsMutex);
  g_unicodeFallbackApps.insert(key);
}

enum class SrfTsfLogLevel {
  Off = 0,
  Error = 1,
  Basic = 2,
  Perf = 3,
  Verbose = 4,
};

std::filesystem::path SrfTsfTraceLogPath() {
  wchar_t localAppData[MAX_PATH] = {};
  const DWORD len = GetEnvironmentVariableW(L"LOCALAPPDATA", localAppData, MAX_PATH);
  if (len == 0 || len >= MAX_PATH) return {};
  return std::filesystem::path(localAppData) / L"kaixin" / L"logs" / L"tsf.log";
}

std::filesystem::path SrfTsfConfigPath() {
  wchar_t localAppData[MAX_PATH] = {};
  const DWORD len = GetEnvironmentVariableW(L"LOCALAPPDATA", localAppData, MAX_PATH);
  if (len == 0 || len >= MAX_PATH) return {};
  return std::filesystem::path(localAppData) / L"kaixin" / L"kaixin.ini";
}

std::wstring LowerAsciiLogValue(std::wstring value) {
  for (wchar_t& ch : value) {
    if (ch >= L'A' && ch <= L'Z') ch = static_cast<wchar_t>(ch - L'A' + L'a');
  }
  return value;
}

bool ParseLogLevel(const std::wstring& raw, SrfTsfLogLevel* out) {
  if (!out) return false;
  const std::wstring value = LowerAsciiLogValue(raw);
  if (value == L"off" || value == L"0" || value == L"false") {
    *out = SrfTsfLogLevel::Off;
    return true;
  }
  if (value == L"error" || value == L"err") {
    *out = SrfTsfLogLevel::Error;
    return true;
  }
  if (value == L"basic" || value == L"info" || value == L"1" || value == L"true" ||
      value == L"on") {
    *out = SrfTsfLogLevel::Basic;
    return true;
  }
  if (value == L"perf" || value == L"performance") {
    *out = SrfTsfLogLevel::Perf;
    return true;
  }
  if (value == L"verbose" || value == L"debug" || value == L"trace") {
    *out = SrfTsfLogLevel::Verbose;
    return true;
  }
  return false;
}

SrfTsfLogLevel ReadConfiguredLogLevel() {
  wchar_t envValue[32] = {};
  if (GetEnvironmentVariableW(L"SRF_IME_LOG_LEVEL", envValue,
                              static_cast<DWORD>(sizeof(envValue) / sizeof(envValue[0]))) > 0) {
    SrfTsfLogLevel level = SrfTsfLogLevel::Basic;
    if (ParseLogLevel(envValue, &level)) return level;
  }
  if (GetEnvironmentVariableW(L"SRF_TSF_DEBUG", envValue,
                              static_cast<DWORD>(sizeof(envValue) / sizeof(envValue[0]))) > 0) {
    SrfTsfLogLevel level = SrfTsfLogLevel::Basic;
    if (ParseLogLevel(envValue, &level)) return level;
    return SrfTsfLogLevel::Verbose;
  }

  const auto configPath = SrfTsfConfigPath();
  if (!configPath.empty()) {
    wchar_t value[32] = {};
    GetPrivateProfileStringW(L"diagnostics", L"log_level", L"", value,
                             static_cast<DWORD>(sizeof(value) / sizeof(value[0])),
                             configPath.c_str());
    SrfTsfLogLevel level = SrfTsfLogLevel::Basic;
    if (ParseLogLevel(value, &level)) return level;
  }
  return SrfTsfLogLevel::Basic;
}

std::atomic<int> g_cachedTsfLogLevel{-1};

void RefreshCachedLogLevel() {
  g_cachedTsfLogLevel.store(static_cast<int>(ReadConfiguredLogLevel()), std::memory_order_release);
}

void MaybeRefreshCachedLogLevel() {
  static ULONGLONG lastRefreshTick = 0;
  const ULONGLONG now = GetTickCount64();
  if (lastRefreshTick != 0 && now >= lastRefreshTick && now - lastRefreshTick < 1000) return;
  lastRefreshTick = now;
  RefreshCachedLogLevel();
}

SrfTsfLogLevel EffectiveLogLevel() {
  int cached = g_cachedTsfLogLevel.load(std::memory_order_acquire);
  if (cached < 0) {
    const int loaded = static_cast<int>(ReadConfiguredLogLevel());
    int expected = -1;
    if (g_cachedTsfLogLevel.compare_exchange_strong(expected, loaded, std::memory_order_acq_rel,
                                                    std::memory_order_acquire)) {
      cached = loaded;
    } else {
      cached = expected;
    }
  }
  return static_cast<SrfTsfLogLevel>(cached);
}

bool SrfTsfLogEnabled(SrfTsfLogLevel level) {
  const SrfTsfLogLevel configured = EffectiveLogLevel();
  return configured != SrfTsfLogLevel::Off &&
         static_cast<int>(level) <= static_cast<int>(configured);
}

void AppendWideUtf8TextRaw(const std::filesystem::path& path, const std::wstring& text) {
  if (path.empty() || text.empty()) return;
  std::error_code ec;
  std::filesystem::create_directories(path.parent_path(), ec);

  const int bytes = WideCharToMultiByte(CP_UTF8, 0, text.c_str(), static_cast<int>(text.size()),
                                        nullptr, 0, nullptr, nullptr);
  if (bytes <= 0) return;

  std::string utf8(static_cast<size_t>(bytes), '\0');
  WideCharToMultiByte(CP_UTF8, 0, text.c_str(), static_cast<int>(text.size()), utf8.data(), bytes,
                      nullptr, nullptr);

  HANDLE file = CreateFileW(path.c_str(), FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE,
                            nullptr, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) return;

  DWORD written = 0;
  WriteFile(file, utf8.data(), static_cast<DWORD>(utf8.size()), &written, nullptr);
  CloseHandle(file);
}

void RotateTraceLogIfNeeded(const std::filesystem::path& path) {
  if (path.empty()) return;
  std::error_code ec;
  const auto size = std::filesystem::file_size(path, ec);
  if (ec || size < kTsfTraceLogMaxBytes) return;

  auto rotatedPath = [&](int index) {
    std::wstring fileName = path.stem().wstring();
    fileName += L".previous";
    if (index > 1) {
      fileName += L".";
      fileName += std::to_wstring(index);
    }
    fileName += path.extension().wstring();
    return path.parent_path() / fileName;
  };

  std::filesystem::remove(rotatedPath(kTsfTraceLogRotateKeep), ec);
  ec.clear();
  for (int index = kTsfTraceLogRotateKeep; index >= 2; --index) {
    const auto from = rotatedPath(index - 1);
    const auto to = rotatedPath(index);
    if (std::filesystem::exists(from, ec)) {
      ec.clear();
      std::filesystem::rename(from, to, ec);
    }
    ec.clear();
  }

  const auto backup = rotatedPath(1);
  std::filesystem::rename(path, backup, ec);
  if (ec) {
    ec.clear();
    std::filesystem::remove(path, ec);
  }
}

std::wstring FormatTraceTimestamp(const SYSTEMTIME& st) {
  wchar_t buf[64] = {};
  swprintf_s(buf, L"%04u-%02u-%02u %02u:%02u:%02u.%03u", static_cast<unsigned>(st.wYear),
              static_cast<unsigned>(st.wMonth), static_cast<unsigned>(st.wDay),
              static_cast<unsigned>(st.wHour), static_cast<unsigned>(st.wMinute),
              static_cast<unsigned>(st.wSecond), static_cast<unsigned>(st.wMilliseconds));
  return buf;
}

std::wstring TrimDiagnosticToken(std::wstring token) {
  while (!token.empty() && (token.front() == L',' || token.front() == L';' || token.front() == L' ')) {
    token.erase(token.begin());
  }
  while (!token.empty() && (token.back() == L',' || token.back() == L';' || token.back() == L' ')) {
    token.pop_back();
  }
  return token;
}

std::wstring LowerAsciiForDiagnostics(std::wstring value) {
  for (wchar_t& ch : value) {
    if (ch >= L'A' && ch <= L'Z') ch = static_cast<wchar_t>(ch - L'A' + L'a');
  }
  return value;
}

bool IsSafeDiagnosticKey(const std::wstring& key) {
  const std::wstring lower = LowerAsciiForDiagnostics(key);
  static constexpr const wchar_t* kSafeKeys[] = {
      L"anchor",       L"anchored",      L"appcontainer", L"async",
      L"class",
      L"candidateempty", L"compat",      L"compathide",   L"composing",
      L"contextempty", L"count",         L"current",      L"currentserial",
      L"cursor",       L"delayms",       L"direct",      L"dwflags",
      L"elapsed",      L"elapsed_ms",    L"engine",       L"fallback",
      L"flags",        L"full",          L"generation",    L"grace_ms_left", L"hasanchor",
      L"hasrect",      L"hr",            L"immersive",    L"integrity",
      L"items",        L"lookup_status",
      L"offset",       L"page",          L"pages",        L"partial",
      L"pid",          L"prefix_placeholder",
      L"process",      L"reason",        L"request_id",   L"refreshcandidates",
      L"result",       L"retained",      L"retry",        L"raw_fallback_suppressed",
      L"secure",
      L"selected",
      L"selectedinpage", L"sensitive",   L"show",         L"shown",
      L"showwindow",   L"stage",         L"status",
      L"state",        L"tid",           L"total",        L"uielement",
      L"uielementid",  L"uiless",        L"uilessmode",   L"visible"};
  for (const wchar_t* safe : kSafeKeys) {
    if (lower == safe) return true;
  }
  return false;
}

std::wstring RedactDiagnosticMessage(const wchar_t* msg) {
  if (!msg || !*msg) return L"(empty)";

  std::wstring text(msg);
  std::wstring out = L"redacted chars=" + std::to_wstring(text.size());
  size_t kept = 0;
  size_t pos = 0;
  while (pos < text.size() && kept < 16) {
    while (pos < text.size() && (text[pos] == L' ' || text[pos] == L',' || text[pos] == L';')) {
      ++pos;
    }
    const size_t start = pos;
    while (pos < text.size() && text[pos] != L' ' && text[pos] != L',' && text[pos] != L';') {
      ++pos;
    }
    if (start == pos) continue;

    std::wstring token = TrimDiagnosticToken(text.substr(start, pos - start));
    const size_t eq = token.find(L'=');
    if (eq == std::wstring::npos || eq == 0) continue;

    const std::wstring key = TrimDiagnosticToken(token.substr(0, eq));
    if (!IsSafeDiagnosticKey(key)) continue;

    out.push_back(L' ');
    out += token;
    ++kept;
  }
  return out;
}


enum class TsfAsyncLogKind : unsigned char {
  Debug,
  Diagnostic,
  Perf,
};

struct PendingTsfLog {
  SYSTEMTIME createdAt = {};
  SrfTsfLogLevel level = SrfTsfLogLevel::Basic;
  TsfAsyncLogKind kind = TsfAsyncLogKind::Diagnostic;
  std::wstring tag;
  std::wstring message;
};

constexpr size_t kTsfLogQueueCapacity = 4096;
constexpr size_t kTsfLogBatchMax = 512;
constexpr auto kTsfLogBatchDelay = std::chrono::milliseconds(50);
constexpr auto kTsfLogWorkerIdleTimeout = std::chrono::seconds(2);

std::mutex g_tsfAsyncLogMutex;
std::condition_variable g_tsfAsyncLogWake;
std::deque<PendingTsfLog> g_tsfAsyncLogQueue;
bool g_tsfAsyncLogWorkerRunning = false;
unsigned long long g_tsfDroppedPerfLogs = 0;

void FlushTsfLogBatch(const std::vector<PendingTsfLog>& batch) {
  if (batch.empty()) return;
  MaybeRefreshCachedLogLevel();

  const auto path = SrfTsfTraceLogPath();
  if (!path.empty()) {
    RotateTraceLogIfNeeded(path);
    std::wstring text;
    text.reserve(batch.size() * 160);
    for (const auto& pending : batch) {
      const std::wstring redacted = RedactDiagnosticMessage(pending.message.c_str());
      text += FormatTraceTimestamp(pending.createdAt);
      text += L" [";
      text += pending.tag.empty() ? L"trace" : pending.tag;
      text += L"] ";
      text += redacted;
      text += L"\r\n";
    }
    AppendWideUtf8TextRaw(path, text);
  }

  if (EffectiveLogLevel() != SrfTsfLogLevel::Verbose) return;
  for (const auto& pending : batch) {
    const std::wstring redacted = RedactDiagnosticMessage(pending.message.c_str());
    switch (pending.kind) {
      case TsfAsyncLogKind::Debug:
        OutputDebugStringW(L"[SRF_TSF] ");
        break;
      case TsfAsyncLogKind::Diagnostic:
        OutputDebugStringW(L"[SRF_TSF_DIAG] ");
        OutputDebugStringW(pending.tag.empty() ? L"diag" : pending.tag.c_str());
        OutputDebugStringW(L": ");
        break;
      case TsfAsyncLogKind::Perf:
        OutputDebugStringW(L"[SRF_TSF_PERF] ");
        OutputDebugStringW(pending.tag.empty() ? L"perf" : pending.tag.c_str());
        OutputDebugStringW(L": ");
        break;
    }
    OutputDebugStringW(redacted.c_str());
    OutputDebugStringW(L"\r\n");
  }
}

void TsfAsyncLogWorkerMain() {
  try {
    for (;;) {
      std::vector<PendingTsfLog> batch;
      {
        std::unique_lock<std::mutex> lock(g_tsfAsyncLogMutex);
        if (g_tsfAsyncLogQueue.empty()) {
          const bool ready = g_tsfAsyncLogWake.wait_for(
              lock, kTsfLogWorkerIdleTimeout, [] { return !g_tsfAsyncLogQueue.empty(); });
          if (!ready && g_tsfAsyncLogQueue.empty()) {
            g_tsfAsyncLogWorkerRunning = false;
            break;
          }
        }
        if (g_tsfAsyncLogQueue.size() < kTsfLogBatchMax) {
          g_tsfAsyncLogWake.wait_for(lock, kTsfLogBatchDelay,
                                     [] { return g_tsfAsyncLogQueue.size() >= kTsfLogBatchMax; });
        }
        const size_t count = std::min(g_tsfAsyncLogQueue.size(), kTsfLogBatchMax);
        batch.reserve(count);
        for (size_t index = 0; index < count; ++index) {
          batch.push_back(std::move(g_tsfAsyncLogQueue.front()));
          g_tsfAsyncLogQueue.pop_front();
        }
      }
      FlushTsfLogBatch(batch);
    }
  } catch (...) {
    std::lock_guard<std::mutex> lock(g_tsfAsyncLogMutex);
    g_tsfAsyncLogWorkerRunning = false;
  }
  SrfTip_BackgroundWorkerRelease();
}

void StartTsfAsyncLogWorker() {
  SrfTip_BackgroundWorkerAddRef();
  try {
    std::thread([] { TsfAsyncLogWorkerMain(); }).detach();
  } catch (...) {
    {
      std::lock_guard<std::mutex> lock(g_tsfAsyncLogMutex);
      g_tsfAsyncLogWorkerRunning = false;
    }
    SrfTip_BackgroundWorkerRelease();
  }
}

void QueueTsfTraceLog(SrfTsfLogLevel level, TsfAsyncLogKind kind, const wchar_t* tag,
                      const wchar_t* msg) {
  bool startWorker = false;
  {
    std::lock_guard<std::mutex> lock(g_tsfAsyncLogMutex);
    if (g_tsfAsyncLogQueue.size() >= kTsfLogQueueCapacity) {
      if (level >= SrfTsfLogLevel::Perf) {
        ++g_tsfDroppedPerfLogs;
        return;
      }
      const auto discard = std::find_if(
          g_tsfAsyncLogQueue.begin(), g_tsfAsyncLogQueue.end(),
          [](const PendingTsfLog& pending) { return pending.level >= SrfTsfLogLevel::Perf; });
      if (discard != g_tsfAsyncLogQueue.end()) {
        g_tsfAsyncLogQueue.erase(discard);
        ++g_tsfDroppedPerfLogs;
      } else {
        g_tsfAsyncLogQueue.pop_front();
      }
    }

    PendingTsfLog pending;
    GetLocalTime(&pending.createdAt);
    pending.level = level;
    pending.kind = kind;
    pending.tag = (tag && *tag) ? tag : L"trace";
    pending.message = (msg && *msg) ? msg : L"(empty)";
    g_tsfAsyncLogQueue.push_back(std::move(pending));
    if (!g_tsfAsyncLogWorkerRunning) {
      g_tsfAsyncLogWorkerRunning = true;
      startWorker = true;
    }
  }
  if (startWorker) StartTsfAsyncLogWorker();
  g_tsfAsyncLogWake.notify_one();
}

}  // namespace

// 供 key_sink.cpp 等翻译单元使用的调试日志工具。
bool SrfTsfDebugTraceEnabled() {
  return SrfTsfLogEnabled(SrfTsfLogLevel::Verbose);
}

void SrfTsfDebugLog(const wchar_t* msg) {
  if (!SrfTsfLogEnabled(SrfTsfLogLevel::Verbose)) return;
  QueueTsfTraceLog(SrfTsfLogLevel::Verbose, TsfAsyncLogKind::Debug, L"debug", msg);
}

void SrfTsfDiagnosticLog(const wchar_t* tag, const wchar_t* msg) {
  if (!SrfTsfLogEnabled(SrfTsfLogLevel::Basic)) return;
  QueueTsfTraceLog(SrfTsfLogLevel::Basic, TsfAsyncLogKind::Diagnostic, tag, msg);
}

void SrfTsfPerfLog(const wchar_t* tag, const wchar_t* msg) {
  if (!SrfTsfLogEnabled(SrfTsfLogLevel::Perf)) return;
  QueueTsfTraceLog(SrfTsfLogLevel::Perf, TsfAsyncLogKind::Perf, tag, msg);
}

namespace {

constexpr DWORD kRustModeFuzzy = 0x0001;
constexpr DWORD kRustModeDouble = 0x0002;
constexpr DWORD kRustModeJianpin = 0x0004;
constexpr DWORD kRustModeVAssist = 0x0008;
constexpr DWORD kRustModeUMode = 0x0010;
constexpr DWORD kRustModeDateAutoFormat = 0x0020;
constexpr DWORD kRustModeClipboardBackground = 0x0040;
constexpr DWORD kRustModeMixedPinyin = 0x0080;
constexpr DWORD kRustModeMixedPinyinAggressive = 0x0100;
constexpr DWORD kRustModeTraditionalOutput = 0x0200;
constexpr DWORD kRustModeEnglishWordInput = 0x0400;
constexpr DWORD kRustModeClipboardDisabled = 0x0800;
constexpr DWORD kRustModeUserPhraseComposeActive = 0x1000;
constexpr DWORD kRustModeSymbolToolbox = 0x2000;
constexpr DWORD kRustModeEmojiInput = 0x4000;
constexpr DWORD kRustModeLearningAggressive = 0x8000;
constexpr DWORD kRustModeLearningConservative = 0x10000;
constexpr wchar_t kStateRegPath[] = L"Software\\kaixin\\State";
constexpr wchar_t kStateAsciiValue[] = L"AsciiMode";
constexpr wchar_t kStateInputAsciiValue[] = L"InputAsciiMode";
constexpr wchar_t kStateInputModeSourceValue[] = L"InputModeSource";
constexpr wchar_t kStateFullShapeValue[] = L"FullShape";
constexpr wchar_t kStateChinesePunctuationValue[] = L"ChinesePunctuation";
constexpr wchar_t kStateInstallMaintenanceValue[] = L"InstallMaintenance";
constexpr wchar_t kTrayWindowClass[] = L"KaixinImeTrayWindow";
constexpr wchar_t kTrayWindowTitle[] = L"\u5f00\u5fc3\u8f93\u5165\u6cd5\u6258\u76d8";
constexpr UINT kTrayStateChangedMessage = WM_APP + 18;
constexpr wchar_t kTrayMutexName[] = L"Local\\KaixinInput_Tray_Mutex";
constexpr ULONGLONG kImeTogglePreservedKeyGuardMs = 600;

void DebugLogPerfMs(const wchar_t* stage, ULONGLONG startTick) {
  if (!SrfTsfLogEnabled(SrfTsfLogLevel::Perf)) return;
  const ULONGLONG elapsed = GetTickCount64() - startTick;
  wchar_t line[192] = {};
  swprintf_s(line, L"stage=%s elapsed_ms=%llu", stage ? stage : L"(none)",
             static_cast<unsigned long long>(elapsed));
  QueueTsfTraceLog(SrfTsfLogLevel::Perf, TsfAsyncLogKind::Perf, L"perf", line);
}

const wchar_t* EngineStateName(SrfEngineState state) {
  switch (state) {
    case SrfEngineState::Idle:
      return L"Idle";
    case SrfEngineState::Loading:
      return L"Loading";
    case SrfEngineState::Ready:
      return L"Ready";
    case SrfEngineState::Failed:
      return L"Failed";
  }
  return L"Unknown";
}

void NotifyTrayAsciiStateChanged(bool asciiMode) {
  HWND tray = FindWindowW(kTrayWindowClass, kTrayWindowTitle);
  if (!tray) return;
  PostMessageW(tray, kTrayStateChangedMessage, asciiMode ? 1 : 0, 0);
}

void NotifyTrayStateChanged() {
  HWND tray = FindWindowW(kTrayWindowClass, kTrayWindowTitle);
  if (!tray) return;
  PostMessageW(tray, kTrayStateChangedMessage, 2, 0);
}

bool WriteStateDwordValue(const wchar_t* name, DWORD value) {
  HKEY key = nullptr;
  if (RegCreateKeyExW(HKEY_CURRENT_USER, kStateRegPath, 0, nullptr, 0, KEY_SET_VALUE, nullptr,
                      &key, nullptr) != ERROR_SUCCESS) {
    return false;
  }
  const bool wrote =
      RegSetValueExW(key, name, 0, REG_DWORD, reinterpret_cast<const BYTE*>(&value),
                     sizeof(value)) == ERROR_SUCCESS;
  RegCloseKey(key);
  return wrote;
}

bool WriteStateStringValue(const wchar_t* name, const std::wstring& value) {
  HKEY key = nullptr;
  if (RegCreateKeyExW(HKEY_CURRENT_USER, kStateRegPath, 0, nullptr, 0, KEY_SET_VALUE, nullptr,
                      &key, nullptr) != ERROR_SUCCESS) {
    return false;
  }
  const DWORD byteCount = static_cast<DWORD>((value.size() + 1) * sizeof(wchar_t));
  const bool wrote = RegSetValueExW(key, name, 0, REG_SZ,
                                    reinterpret_cast<const BYTE*>(value.c_str()),
                                    byteCount) == ERROR_SUCCESS;
  RegCloseKey(key);
  return wrote;
}

std::wstring ProcessNameForWindow(HWND hwnd, DWORD* processIdOut = nullptr) {
  if (processIdOut) *processIdOut = 0;
  if (!hwnd) return {};

  DWORD processId = 0;
  (void)GetWindowThreadProcessId(hwnd, &processId);
  if (processIdOut) *processIdOut = processId;
  if (processId == 0) return {};

  HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, processId);
  if (!process) return {};

  std::wstring path(MAX_PATH, L'\0');
  DWORD size = static_cast<DWORD>(path.size());
  std::wstring name;
  if (QueryFullProcessImageNameW(process, 0, path.data(), &size) && size > 0) {
    path.resize(size);
    name = std::move(path);
  }
  CloseHandle(process);
  return name;
}

void PublishTrayInputStatus(bool asciiMode, bool fullShape, bool chinesePunctuation,
                            const std::wstring& modeSource) {
  static bool initialized = false;
  static bool lastAsciiMode = false;
  static bool lastFullShape = false;
  static bool lastChinesePunctuation = true;
  static std::wstring lastModeSource;

  if (initialized && lastAsciiMode == asciiMode && lastFullShape == fullShape &&
      lastChinesePunctuation == chinesePunctuation && lastModeSource == modeSource) {
    return;
  }

  const bool wroteAscii = WriteStateDwordValue(kStateInputAsciiValue, asciiMode ? 1u : 0u);
  const bool wroteFullShape = WriteStateDwordValue(kStateFullShapeValue, fullShape ? 1u : 0u);
  const bool wrotePunctuation =
      WriteStateDwordValue(kStateChinesePunctuationValue, chinesePunctuation ? 1u : 0u);
  const bool wroteModeSource = WriteStateStringValue(kStateInputModeSourceValue, modeSource);

  initialized = true;
  lastAsciiMode = asciiMode;
  lastFullShape = fullShape;
  lastChinesePunctuation = chinesePunctuation;
  lastModeSource = modeSource;

  if (wroteAscii || wroteFullShape || wrotePunctuation || wroteModeSource) {
    NotifyTrayStateChanged();
  }
}

bool InstallMaintenanceActive() {
  DWORD value = 0;
  DWORD cb = sizeof(value);
  return RegGetValueW(HKEY_CURRENT_USER, kStateRegPath, kStateInstallMaintenanceValue,
                      RRF_RT_REG_DWORD, nullptr, &value, &cb) == ERROR_SUCCESS &&
         value != 0;
}

std::wstring ShortenForLog(const std::wstring& text, size_t maxUnits = 48) {
  if (text.size() <= maxUnits) return text;
  if (maxUnits <= 3) return text.substr(0, maxUnits);
  return text.substr(0, maxUnits - 3) + L"...";
}

std::wstring FormatRectForLog(const RECT& rect) {
  wchar_t buf[96] = {};
  swprintf_s(buf, L"(%ld,%ld,%ld,%ld)", rect.left, rect.top, rect.right, rect.bottom);
  return buf;
}

std::wstring FormatPointerForLog(const void* ptr) {
  wchar_t buf[32] = {};
  swprintf_s(buf, L"%p", ptr);
  return buf;
}

bool HasCtrlOrAltDown() {
  return (GetKeyState(VK_CONTROL) & 0x8000) != 0 || (GetKeyState(VK_MENU) & 0x8000) != 0;
}

bool HasCtrlOrShiftDown() {
  return (GetKeyState(VK_CONTROL) & 0x8000) != 0 || (GetKeyState(VK_SHIFT) & 0x8000) != 0;
}

// 注意：VK_F1..VK_F11 的数值与 ASCII 'p'..'z' 重合（均为 0x70..0x7A），
// 若仅按 'a'..'z' 判断会把 F1–F11 当成字母键。
bool IsLetterVk(UINT vk) {
  if (vk >= VK_F1 && vk <= VK_F24) return false;
  return (vk >= 'A' && vk <= 'Z') || (vk >= 'a' && vk <= 'z');
}

bool IsVkShift(UINT vk) { return vk == VK_SHIFT || vk == VK_LSHIFT || vk == VK_RSHIFT; }

bool IsDigitVk(UINT vk) { return vk >= '0' && vk <= '9'; }

bool IsNumpadDigitVk(UINT vk) { return vk >= VK_NUMPAD0 && vk <= VK_NUMPAD9; }

bool IsShiftedNumberRowSymbolVk(UINT vk) {
  return IsDigitVk(vk) && (GetKeyState(VK_SHIFT) & 0x8000) != 0;
}

wchar_t DigitCharFromVk(UINT vk) {
  if (vk >= '0' && vk <= '9') return static_cast<wchar_t>(vk);
  if (vk >= VK_NUMPAD0 && vk <= VK_NUMPAD9) return static_cast<wchar_t>(L'0' + (vk - VK_NUMPAD0));
  return 0;
}

int CandidateNumberIndexFromVk(UINT vk) {
  if (vk >= '1' && vk <= '9') return static_cast<int>(vk - '1');
  if (vk == '0') return 9;
  if (vk >= VK_NUMPAD1 && vk <= VK_NUMPAD9) return static_cast<int>(vk - VK_NUMPAD1);
  if (vk == VK_NUMPAD0) return 9;
  return -1;
}

void TrimWstringInPlace(std::wstring& s) {
  while (!s.empty() && iswspace(s.front())) s.erase(0, 1);
  while (!s.empty() && iswspace(s.back())) s.pop_back();
}

bool IsAsciiAlphaText(const std::wstring& text) {
  return !text.empty() &&
         std::all_of(text.begin(), text.end(), [](wchar_t ch) {
           return (ch >= L'a' && ch <= L'z') || (ch >= L'A' && ch <= L'Z');
         });
}

bool HasPhraseCandidateInTop(const std::vector<std::wstring>& candidates, size_t topN) {
  const size_t n = std::min(topN, candidates.size());
  for (size_t i = 0; i < n; ++i) {
    if (candidates[i].size() >= 2) return true;
  }
  return false;
}

void MaybeInsertPredictedPhraseCandidates(const std::wstring& reading,
                                         std::vector<std::wstring>* candidates,
                                         std::vector<std::wstring>* candidateMeta) {
  if (!candidates || reading.empty()) return;

  // 仅在“多音节 + 缺少短语候选”时插入少量预测组合，避免干扰正常排序。
  constexpr size_t kInspectTopN = 8;
  if (HasPhraseCandidateInTop(*candidates, kInspectTopN)) return;

  std::wstring trimmed = reading;
  TrimWstringInPlace(trimmed);
  if (trimmed.empty()) return;

  std::array<uint32_t, 64> bounds{};
  const size_t n = SrfTip_SyllableBoundaryOffsetsUtf16(trimmed.c_str(), trimmed.size(),
                                                       bounds.data(), bounds.size());
  // bounds 至少包含 [0, end]；多音节需要至少 3 个点：[0, split, end]
  if (n < 3) return;
  const size_t split = static_cast<size_t>(bounds[1]);
  if (split == 0 || split >= trimmed.size()) return;

  const std::wstring restReadingRaw = trimmed.substr(split);
  std::wstring restReading = restReadingRaw;
  TrimWstringInPlace(restReading);
  if (restReading.empty()) return;

  // 从现有候选中取若干“首音节单字”作为组合前缀。
  constexpr size_t kMaxFirstChars = 5;
  std::vector<std::wstring> firstChars;
  firstChars.reserve(kMaxFirstChars);
  std::unordered_set<std::wstring> seenFirst;
  for (const auto& cand : *candidates) {
    if (cand.size() != 1) continue;
    if (seenFirst.insert(cand).second) {
      firstChars.push_back(cand);
      if (firstChars.size() >= kMaxFirstChars) break;
    }
  }
  if (firstChars.empty()) return;

  // 查询剩余 reading 的候选，用于生成组合（不污染主候选状态）。
  std::vector<std::wstring> restCandidates;
  std::vector<std::wstring> restMeta;
  SrfTip_LookupCandidates(restReading, restCandidates, &restMeta);
  if (restCandidates.empty()) return;

  constexpr size_t kMaxRest = 3;
  constexpr size_t kMaxPredicted = 5;
  std::vector<std::wstring> predictedTexts;
  std::vector<std::wstring> predictedMeta;
  predictedTexts.reserve(kMaxPredicted);
  predictedMeta.reserve(kMaxPredicted);

  std::unordered_set<std::wstring> existing(candidates->begin(), candidates->end());
  std::unordered_set<std::wstring> predictedSet;

  for (const auto& first : firstChars) {
    size_t used = 0;
    for (const auto& rest : restCandidates) {
      if (rest.empty()) continue;
      const std::wstring combined = first + rest;
      if (existing.find(combined) != existing.end()) continue;
      if (!predictedSet.insert(combined).second) continue;
      predictedTexts.push_back(combined);
      predictedMeta.push_back(L"预测组合");
      if (!predictedMeta.empty()) predictedMeta.back() = L"no_learn=1\t\u9884\u6d4b\u7ec4\u5408";
      if (++used >= kMaxRest) break;
      if (predictedTexts.size() >= kMaxPredicted) break;
    }
    if (predictedTexts.size() >= kMaxPredicted) break;
  }

  if (predictedTexts.empty()) return;

  if (candidateMeta) {
    if (candidateMeta->size() < candidates->size()) candidateMeta->resize(candidates->size());
    candidateMeta->insert(candidateMeta->begin(), predictedMeta.begin(), predictedMeta.end());
  }
  candidates->insert(candidates->begin(), predictedTexts.begin(), predictedTexts.end());
}

struct CandidateMetaParts {
  std::wstring display;
  std::wstring annotation;
  std::wstring score;
  std::wstring correctedReading;
  bool noLearn = false;
  bool clipboardQuick = false;
  std::wstring clipboardId;
  std::wstring clipboardSource;
  std::wstring clipboardTime;
  std::wstring clipboardType;
  std::wstring clipboardFilter;
  UINT clipboardPage = 1;
  UINT clipboardPages = 1;
  bool clipboardPinned = false;
  bool forceVerticalLayout = false;
  bool partialResult = false;
  bool correctionCandidate = false;
  bool prefixPlaceholder = false;
  bool pinnedExactInput = false;
  bool userCandidate = false;
  std::wstring userSourceLabel;
  bool extCandidate = false;
};

bool LooksLikeNumericCandidateMeta(const std::wstring& text) {
  if (text.empty()) return false;
  return std::all_of(text.begin(), text.end(), [](wchar_t ch) {
    return (ch >= L'0' && ch <= L'9') || ch == L'.' || ch == L'-' || iswspace(ch);
  });
}

bool IsVisibleCandidateAnnotation(const std::wstring& text) {
  constexpr wchar_t kCorrectionPrefix[] = L"\u7ea0\u9519";
  constexpr wchar_t kNearPrefix[] = L"\u97f3\u8fd1";
  return text.rfind(kCorrectionPrefix, 0) == 0 || text.rfind(kNearPrefix, 0) == 0;
}

bool IsStructuredDirectCandidateAnnotation(const std::wstring& text) {
  return text.rfind(L"Emoji:", 0) == 0 || text.rfind(L"\u7b26\u53f7:", 0) == 0;
}

CandidateMetaParts SplitCandidateMeta(const std::wstring& raw) {
  struct CandidateMetaCacheEntry {
    std::wstring raw;
    CandidateMetaParts parts;
    bool valid = false;
  };
  thread_local std::array<CandidateMetaCacheEntry, 64> cache = {};
  thread_local size_t nextCacheSlot = 0;

  for (const auto& entry : cache) {
    if (entry.valid && entry.raw == raw) return entry.parts;
  }

  CandidateMetaParts parts;
  auto appendAnnotation = [&](const std::wstring& token) {
    const bool correctionAnnotation = IsVisibleCandidateAnnotation(token);
    if (!correctionAnnotation && !IsStructuredDirectCandidateAnnotation(token)) return;
    if (correctionAnnotation) parts.correctionCandidate = true;
    if (!parts.annotation.empty()) parts.annotation += L" | ";
    parts.annotation += token;
  };
  size_t start = 0;
  while (start <= raw.size()) {
    const size_t tab = raw.find(L'\t', start);
    std::wstring token =
        raw.substr(start, tab == std::wstring::npos ? std::wstring::npos : tab - start);
    TrimWstringInPlace(token);
    if (!token.empty()) {
      constexpr wchar_t kDisplayPrefix[] = L"display=";
      constexpr size_t kDisplayPrefixLen = 8;
      if (token.rfind(kDisplayPrefix, 0) == 0) {
        parts.display = token.substr(kDisplayPrefixLen);
        TrimWstringInPlace(parts.display);
      } else if (token == L"no_learn" || token == L"no_learn=1" || token == L"learn=0") {
        parts.noLearn = true;
      } else if (token == L"clipboard_quick" || token == L"clipboard_quick=1") {
        parts.clipboardQuick = true;
      } else if (token == L"partial" || token == L"partial=1") {
        parts.partialResult = true;
      } else if (token == L"typo" || token == L"typo=1" || token == L"correction=1" ||
                 token == L"source=correction" || token == L"match=correction" ||
                 token.rfind(L"correction=", 0) == 0) {
        parts.correctionCandidate = true;
      } else if (token == L"source=user_fresh") {
        parts.userCandidate = true;
        parts.userSourceLabel = L"\u7528\u6237\u751f\u8bcd";
      } else if (token == L"mixed_pinyin_user" || token == L"source=user_mixed") {
        parts.userCandidate = true;
        parts.userSourceLabel = L"\u6df7\u62fc\u8bb0\u5fc6";
      } else if (token == L"observed_user" || token == L"source=user_observed") {
        parts.userCandidate = true;
        parts.userSourceLabel = L"\u89c2\u5bdf\u4e2d";
      } else if (token == L"user" || token == L"user=1" || token == L"\u7528\u6237\u8bcd" ||
                 token == L"source=user" || token == L"source=user_common") {
        parts.userCandidate = true;
        if (parts.userSourceLabel.empty()) parts.userSourceLabel = L"\u7528\u6237\u5e38\u7528";
      } else if (token == L"layer=ext" || token == L"layer=large") {
        parts.extCandidate = true;
      } else if (token == L"prefix_placeholder" || token == L"prefix_placeholder=1" ||
                 token == L"placeholder=prefix") {
        parts.prefixPlaceholder = true;
      } else if (token == L"pinned" || token == L"pinned=1" || token == L"pin=1") {
        parts.pinnedExactInput = true;
        parts.userCandidate = true;
      } else if (token.rfind(L"clipboard_key=", 0) == 0) {
        parts.clipboardId = token.substr(14);
        TrimWstringInPlace(parts.clipboardId);
      } else if (token.rfind(L"clipboard_source=", 0) == 0) {
        parts.clipboardSource = token.substr(17);
        TrimWstringInPlace(parts.clipboardSource);
      } else if (token.rfind(L"clipboard_time=", 0) == 0) {
        parts.clipboardTime = token.substr(15);
        TrimWstringInPlace(parts.clipboardTime);
      } else if (token.rfind(L"clipboard_type=", 0) == 0) {
        parts.clipboardType = token.substr(15);
        TrimWstringInPlace(parts.clipboardType);
      } else if (token.rfind(L"clipboard_filter=", 0) == 0) {
        parts.clipboardFilter = token.substr(17);
        TrimWstringInPlace(parts.clipboardFilter);
      } else if (token.rfind(L"clipboard_page=", 0) == 0) {
        const unsigned long page = wcstoul(token.c_str() + 15, nullptr, 10);
        parts.clipboardPage = static_cast<UINT>(std::clamp<unsigned long>(page, 1, 100000));
      } else if (token.rfind(L"clipboard_pages=", 0) == 0) {
        const unsigned long pages = wcstoul(token.c_str() + 16, nullptr, 10);
        parts.clipboardPages = static_cast<UINT>(std::clamp<unsigned long>(pages, 1, 100000));
      } else if (token == L"clipboard_pinned" || token == L"clipboard_pinned=1") {
        parts.clipboardPinned = true;
      } else if (token.rfind(L"corrected_reading=", 0) == 0) {
        parts.correctedReading = token.substr(18);
        TrimWstringInPlace(parts.correctedReading);
      } else if (token == L"layout=vertical" || token == L"force_vertical=1") {
        parts.forceVerticalLayout = true;
      } else if (LooksLikeNumericCandidateMeta(token)) {
        parts.score = token;
      } else {
        appendAnnotation(token);
      }
    }
    if (tab == std::wstring::npos) break;
    start = tab + 1;
  }
  auto& slot = cache[nextCacheSlot++ % cache.size()];
  slot.raw = raw;
  slot.parts = parts;
  slot.valid = true;
  return parts;
}

std::wstring CandidateSourcePrefix(const CandidateMetaParts& meta, const SrfUIStyle& style) {
  if (style.showCandidateSource) {
    if (meta.correctionCandidate) return L"~";
    if (meta.pinnedExactInput) return L"\x2B50 ";
    if (meta.userCandidate) return L"\xD83D\xDD50 ";
    if (meta.extCandidate) return L"\xD83D\xDCD8 ";
    return std::wstring();
  }
  if (style.highlightTypoCandidates && meta.correctionCandidate) return L"~";
  return std::wstring();
}

std::wstring CandidateSourceComment(const CandidateMetaParts& meta) {
  if (meta.correctionCandidate) return L"\u7ea0\u9519";
  if (meta.pinnedExactInput) return L"\u7f6e\u9876";
  if (meta.userCandidate) return L"\u7528\u6237\u8bcd";
  if (meta.extCandidate) return L"\u6269\u5c55\u8bcd\u5e93";
  return {};
}

void PrefixCandidateDisplayText(std::wstring* text, const CandidateMetaParts& meta,
                                const SrfUIStyle& style) {
  if (!text) return;
  const std::wstring prefix = CandidateSourcePrefix(meta, style);
  if (!prefix.empty()) text->insert(0, prefix);
}

void AppendCommentPart(std::wstring* out, const std::wstring& part) {
  if (!out || part.empty()) return;
  if (!out->empty()) *out += L" | ";
  *out += part;
}

bool IsFunctionKeyTokenPrefix(const std::wstring& text) {
  if (text.empty() || text.size() > 3) return false;
  if (text[0] != L'f' && text[0] != L'F') return false;
  for (size_t i = 1; i < text.size(); ++i) {
    if (text[i] < L'0' || text[i] > L'9') return false;
  }
  return true;
}

bool IsValidFunctionKeyToken(const std::wstring& text) {
  if (!IsFunctionKeyTokenPrefix(text) || text.size() == 1) return false;
  unsigned value = 0;
  for (size_t i = 1; i < text.size(); ++i) {
    value = value * 10 + static_cast<unsigned>(text[i] - L'0');
  }
  return value >= 1 && value <= 24;
}

std::wstring LowerAscii(std::wstring text) {
  for (wchar_t& ch : text) {
    if (ch >= L'A' && ch <= L'Z') ch = static_cast<wchar_t>(ch - L'A' + L'a');
  }
  return text;
}

bool TryParseNamedFunctionKeyToken(const std::wstring& text, UINT* outVk) {
  if (!outVk) return false;
  const std::wstring key = LowerAscii(text);
  if (key == L"enter" || key == L"return" || key == L"huiche") {
    *outVk = VK_RETURN;
    return true;
  }
  if (key == L"backspace" || key == L"bksp" || key == L"tuige") {
    *outVk = VK_BACK;
    return true;
  }
  if (key == L"delete" || key == L"del") {
    *outVk = VK_DELETE;
    return true;
  }
  if (key == L"escape" || key == L"esc") {
    *outVk = VK_ESCAPE;
    return true;
  }
  if (key == L"tab") {
    *outVk = VK_TAB;
    return true;
  }
  return false;
}

bool TryParseFunctionKeyToken(const std::wstring& text, UINT* outVk) {
  if (TryParseNamedFunctionKeyToken(text, outVk)) return true;
  if (!outVk || !IsValidFunctionKeyToken(text)) return false;
  unsigned value = 0;
  for (size_t i = 1; i < text.size(); ++i) {
    value = value * 10 + static_cast<unsigned>(text[i] - L'0');
  }
  *outVk = static_cast<UINT>(VK_F1 + value - 1);
  return true;
}

bool SendVirtualKeyTap(UINT vk) {
  const UINT scanCodeEx = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC_EX);
  INPUT inputs[2] = {};
  if (scanCodeEx != 0) {
    const WORD scanCode = static_cast<WORD>(scanCodeEx & 0xff);
    const DWORD extFlags = (scanCodeEx & 0xff00) != 0 ? KEYEVENTF_EXTENDEDKEY : 0;
    inputs[0].type = INPUT_KEYBOARD;
    inputs[0].ki.wScan = scanCode;
    inputs[0].ki.dwFlags = KEYEVENTF_SCANCODE | extFlags;
    inputs[1] = inputs[0];
    inputs[1].ki.dwFlags |= KEYEVENTF_KEYUP;
  } else {
    inputs[0].type = INPUT_KEYBOARD;
    inputs[0].ki.wVk = static_cast<WORD>(vk);
    inputs[1] = inputs[0];
    inputs[1].ki.dwFlags = KEYEVENTF_KEYUP;
  }
  return SendInput(static_cast<UINT>(std::size(inputs)), inputs, sizeof(INPUT)) == std::size(inputs);
}

HRESULT SendCtrlVPaste() {
  INPUT inputs[4] = {};
  inputs[0].type = INPUT_KEYBOARD;
  inputs[0].ki.wVk = VK_CONTROL;
  inputs[1].type = INPUT_KEYBOARD;
  inputs[1].ki.wVk = 'V';
  inputs[2] = inputs[1];
  inputs[2].ki.dwFlags = KEYEVENTF_KEYUP;
  inputs[3] = inputs[0];
  inputs[3].ki.dwFlags = KEYEVENTF_KEYUP;
  if (SendInput(static_cast<UINT>(std::size(inputs)), inputs, sizeof(INPUT)) == std::size(inputs)) {
    return S_OK;
  }
  const DWORD err = GetLastError();
  return HRESULT_FROM_WIN32(err != 0 ? err : ERROR_GEN_FAILURE);
}

class ScopedOleClipboardRestore {
 public:
  ScopedOleClipboardRestore() {
    original_sequence_ = GetClipboardSequenceNumber();
    IDataObject* previous = nullptr;
    if (SUCCEEDED(OleGetClipboard(&previous))) previous_ = previous;
  }

  ScopedOleClipboardRestore(const ScopedOleClipboardRestore&) = delete;
  ScopedOleClipboardRestore& operator=(const ScopedOleClipboardRestore&) = delete;

  ~ScopedOleClipboardRestore() {
    if (previous_) previous_->Release();
  }

  bool HasData() const { return previous_ != nullptr; }

  void MarkTemporaryClipboard(DWORD sequence = 0) {
    temporary_sequence_ = sequence != 0 ? sequence : GetClipboardSequenceNumber();
    temporary_sequence_valid_ = temporary_sequence_ != 0;
  }

  HRESULT RestoreAfterPaste() {
    if (!previous_) return S_FALSE;
    // Give the target a brief chance to consume Ctrl+V, but never restore over
    // content copied by the user while the paste was in flight.
    Sleep(60);
    const DWORD current_sequence = GetClipboardSequenceNumber();
    const DWORD expected_sequence =
        temporary_sequence_valid_ ? temporary_sequence_ : original_sequence_;
    if (expected_sequence != 0 && current_sequence != 0 &&
        current_sequence != expected_sequence) {
      SrfTsfDiagnosticLog(L"clipboard-paste.restore",
                          L"status=skipped reason=clipboard_changed_during_paste");
      previous_->Release();
      previous_ = nullptr;
      return S_FALSE;
    }
    HRESULT hr = OleSetClipboard(previous_);
    if (SUCCEEDED(hr)) {
      HRESULT flushHr = OleFlushClipboard();
      if (FAILED(flushHr)) hr = flushHr;
    }
    previous_->Release();
    previous_ = nullptr;
    return hr;
  }

 private:
  IDataObject* previous_ = nullptr;
  DWORD original_sequence_ = 0;
  DWORD temporary_sequence_ = 0;
  bool temporary_sequence_valid_ = false;
};

UINT TemporaryPasteClipboardFormat() {
  static UINT format = RegisterClipboardFormatW(L"KaixinInput.TemporaryPaste");
  return format;
}

HWND ClipboardOwnerWindow() {
  static HWND owner = nullptr;
  if (owner && IsWindow(owner)) return owner;

  constexpr wchar_t kClassName[] = L"SrfTsfClipboardOwner";
  WNDCLASSW wc = {};
  wc.lpfnWndProc = DefWindowProcW;
  wc.hInstance = GetModuleHandleW(nullptr);
  wc.lpszClassName = kClassName;
  if (!RegisterClassW(&wc) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) {
    return nullptr;
  }

  owner = CreateWindowExW(0, kClassName, L"", 0, 0, 0, 0, 0, HWND_MESSAGE,
                          nullptr, wc.hInstance, nullptr);
  return owner;
}

HRESULT SetUnicodeClipboardTextForPaste(const std::wstring& text,
                                        DWORD* out_sequence = nullptr) {
  HWND owner = ClipboardOwnerWindow();
  if (!owner) owner = GetForegroundWindow();
  bool opened = false;
  for (int attempt = 0; attempt < 24; ++attempt) {
    if (OpenClipboard(owner)) {
      opened = true;
      break;
    }
    Sleep(8 + static_cast<DWORD>(attempt));
  }
  if (!opened) {
    const DWORD err = GetLastError();
    return HRESULT_FROM_WIN32(err != 0 ? err : ERROR_ACCESS_DENIED);
  }

  const SIZE_T bytes = (text.size() + 1) * sizeof(wchar_t);
  HGLOBAL memory = GlobalAlloc(GMEM_MOVEABLE, bytes);
  if (!memory) {
    CloseClipboard();
    return HRESULT_FROM_WIN32(GetLastError() != 0 ? GetLastError() : ERROR_OUTOFMEMORY);
  }

  void* locked = GlobalLock(memory);
  if (!locked) {
    const DWORD err = GetLastError();
    GlobalFree(memory);
    CloseClipboard();
    return HRESULT_FROM_WIN32(err != 0 ? err : ERROR_LOCK_FAILED);
  }
  std::copy(text.c_str(), text.c_str() + text.size() + 1, static_cast<wchar_t*>(locked));
  GlobalUnlock(memory);

  HRESULT hr = S_OK;
  if (!EmptyClipboard()) {
    const DWORD err = GetLastError();
    hr = HRESULT_FROM_WIN32(err != 0 ? err : ERROR_ACCESS_DENIED);
  } else if (!SetClipboardData(CF_UNICODETEXT, memory)) {
    const DWORD err = GetLastError();
    hr = HRESULT_FROM_WIN32(err != 0 ? err : ERROR_ACCESS_DENIED);
  } else {
    memory = nullptr;
    const UINT markerFormat = TemporaryPasteClipboardFormat();
    if (markerFormat != 0) {
      HGLOBAL marker = GlobalAlloc(GMEM_MOVEABLE, sizeof(DWORD));
      if (marker) {
        void* markerLocked = GlobalLock(marker);
        if (markerLocked) {
          *static_cast<DWORD*>(markerLocked) = 1;
          GlobalUnlock(marker);
        }
        if (!SetClipboardData(markerFormat, marker)) {
          GlobalFree(marker);
        }
      }
    }
  }

  if (memory) GlobalFree(memory);
  CloseClipboard();
  if (out_sequence) *out_sequence = GetClipboardSequenceNumber();
  // Let the clipboard owner publish both CF_UNICODETEXT and the temporary
  // marker before the listener receives WM_CLIPBOARDUPDATE.
  Sleep(10);
  return hr;
}

HRESULT PasteUnicodeTextViaClipboard(const std::wstring& text) {
  ScopedOleClipboardRestore restore;
  DWORD temporary_sequence = 0;
  HRESULT hr = SetUnicodeClipboardTextForPaste(text, &temporary_sequence);
  if (SUCCEEDED(hr)) restore.MarkTemporaryClipboard(temporary_sequence);
  if (SUCCEEDED(hr)) hr = SendCtrlVPaste();
  if (restore.HasData()) {
    const HRESULT restoreHr = restore.RestoreAfterPaste();
    if (FAILED(restoreHr)) {
      std::wstring line = L"status=restore_failed, hr=0x";
      wchar_t hrBuf[16] = {};
      swprintf_s(hrBuf, L"%08lX", static_cast<unsigned long>(restoreHr));
      line += hrBuf;
      SrfTsfDiagnosticLog(L"clipboard-paste.restore", line.c_str());
    }
  }
  return hr;
}

HRESULT SendUnicodeTextInput(const std::wstring& text) {
  if (text.empty()) return S_OK;
  std::vector<INPUT> inputs;
  inputs.reserve(text.size() * 2);
  for (wchar_t ch : text) {
    INPUT down = {};
    down.type = INPUT_KEYBOARD;
    down.ki.wScan = static_cast<WORD>(ch);
    down.ki.dwFlags = KEYEVENTF_UNICODE;
    INPUT up = down;
    up.ki.dwFlags = KEYEVENTF_UNICODE | KEYEVENTF_KEYUP;
    inputs.push_back(down);
    inputs.push_back(up);
  }
  const UINT sent =
      SendInput(static_cast<UINT>(inputs.size()), inputs.data(), sizeof(INPUT));
  if (sent == inputs.size()) return S_OK;
  const DWORD err = GetLastError();
  return HRESULT_FROM_WIN32(err != 0 ? err : ERROR_GEN_FAILURE);
}

bool UnicodeInputTargetStillAlive(HWND hwnd, DWORD processId) {
  if (!hwnd || processId == 0) return true;
  if (!IsWindow(hwnd)) return false;
  DWORD currentProcessId = 0;
  (void)GetWindowThreadProcessId(hwnd, &currentProcessId);
  if (currentProcessId != processId) return false;
  return IsWindowVisible(hwnd) != FALSE;
}

bool IsNumpadPrintableVk(UINT vk) {
  switch (vk) {
    case VK_ADD:
    case VK_SUBTRACT:
    case VK_MULTIPLY:
    case VK_DIVIDE:
    case VK_DECIMAL:
    case VK_SEPARATOR:
      return true;
    default:
      return IsNumpadDigitVk(vk);
  }
}

bool IsOemPrintableVk(UINT vk) {
  switch (vk) {
    case VK_OEM_1:
    case VK_OEM_2:
    case VK_OEM_3:
    case VK_OEM_4:
    case VK_OEM_5:
    case VK_OEM_6:
    case VK_OEM_7:
    case VK_OEM_PLUS:
    case VK_OEM_COMMA:
    case VK_OEM_MINUS:
    case VK_OEM_PERIOD:
    case VK_OEM_102:
      return true;
    default:
      return false;
  }
}

bool IsPrintableDirectVk(UINT vk) {
  return IsLetterVk(vk) || IsDigitVk(vk) || IsNumpadPrintableVk(vk) || IsOemPrintableVk(vk) ||
         vk == VK_SPACE;
}

std::wstring TrimAsciiWhitespace(std::wstring value) {
  const auto first = value.find_first_not_of(L" \t\r\n");
  if (first == std::wstring::npos) return {};
  const auto last = value.find_last_not_of(L" \t\r\n");
  return value.substr(first, last - first + 1);
}

bool IsClipboardQuickMeta(const std::wstring& meta) {
  return meta.find(L"clipboard_quick") != std::wstring::npos;
}

bool ParseClipboardQuickPageToken(const std::wstring& token, UINT* outPage) {
  if (!outPage || token.empty()) return false;
  size_t pos = 0;
  if (token[0] == L'p' || token[0] == L'P') pos = 1;
  if (pos >= token.size()) return false;
  UINT pageOneBased = 0;
  for (; pos < token.size(); ++pos) {
    const wchar_t ch = token[pos];
    if (ch < L'0' || ch > L'9') return false;
    pageOneBased = pageOneBased * 10 + static_cast<UINT>(ch - L'0');
  }
  if (pageOneBased == 0) return false;
  *outPage = pageOneBased - 1;
  return true;
}

bool ParseClipboardQuickReading(const std::wstring& reading, UINT* outPage,
                                std::wstring* outFilter) {
  if (reading.size() < 3 || reading.compare(0, 3, L"vvu") != 0) return false;

  UINT page = 0;
  std::wstring filter;
  std::wstring rest;
  if (reading.size() > 3) {
    if (reading[3] == L' ' || reading[3] == L'\t') {
      rest = TrimAsciiWhitespace(reading.substr(3));
    } else if (reading[3] >= L'0' && reading[3] <= L'9') {
      size_t pos = 3;
      while (pos < reading.size() && reading[pos] >= L'0' && reading[pos] <= L'9') ++pos;
      if (!ParseClipboardQuickPageToken(reading.substr(3, pos - 3), &page)) return false;
      filter = TrimAsciiWhitespace(reading.substr(pos));
    } else {
      return false;
    }
  }
  if (!rest.empty()) {
    const auto split = rest.find_first_of(L" \t\r\n");
    const std::wstring first =
        split == std::wstring::npos ? rest : rest.substr(0, split);
    const std::wstring tail =
        split == std::wstring::npos ? L"" : TrimAsciiWhitespace(rest.substr(split + 1));
    if (!first.empty() && (first[0] == L'p' || first[0] == L'P') &&
        ParseClipboardQuickPageToken(first, &page)) {
      filter = tail;
    } else {
      filter = rest;
    }
  }
  if (outPage) *outPage = page;
  if (outFilter) *outFilter = std::move(filter);
  return true;
}

std::wstring BuildClipboardQuickReading(UINT page, const std::wstring& filter) {
  if (page == 0 && filter.empty()) return L"vvu";
  if (page > 0 && filter.empty()) return L"vvu" + std::to_wstring(page + 1);
  std::wstring out = L"vvu ";
  if (page > 0) {
    out += filter.empty() ? std::to_wstring(page + 1) : (L"p" + std::to_wstring(page + 1));
    if (!filter.empty()) out += L" ";
  }
  out += filter;
  return out;
}

bool IsAsciiPunctuationChar(wchar_t ch) {
  return ch >= 0x21 && ch <= 0x7e && !iswalnum(static_cast<wint_t>(ch));
}

bool IsDirectInsertPreferredChar(wchar_t ch) {
  if ((ch >= L'0' && ch <= L'9') || ch == L' ') return true;
  if (IsAsciiPunctuationChar(ch)) return true;
  // 全角 ASCII（含全角数字/符号）
  if (ch >= 0xFF01 && ch <= 0xFF5E) return true;
  // 常见中文标点及空格
  switch (ch) {
    case L'，':
    case L'。':
    case L'？':
    case L'！':
    case L'；':
    case L'：':
    case L'（':
    case L'）':
    case L'【':
    case L'】':
    case L'《':
    case L'》':
    case L'、':
    case L'／':
    case L'“':
    case L'”':
    case L'‘':
    case L'’':
    case L'　':
      return true;
    default:
      return false;
  }
}

std::wstring FallbackPrintableText(UINT vk, bool shiftDown, bool capsLockOn) {
  switch (vk) {
    case 'A':
    case 'B':
    case 'C':
    case 'D':
    case 'E':
    case 'F':
    case 'G':
    case 'H':
    case 'I':
    case 'J':
    case 'K':
    case 'L':
    case 'M':
    case 'N':
    case 'O':
    case 'P':
    case 'Q':
    case 'R':
    case 'S':
    case 'T':
    case 'U':
    case 'V':
    case 'W':
    case 'X':
    case 'Y':
    case 'Z': {
      const bool upper = shiftDown != capsLockOn;
      const wchar_t ch = static_cast<wchar_t>(upper ? vk : (vk - 'A' + 'a'));
      return std::wstring(1, ch);
    }
    case VK_SPACE:
      return L" ";
    case '0':
      return std::wstring(1, shiftDown ? L')' : L'0');
    case '1':
      return std::wstring(1, shiftDown ? L'!' : L'1');
    case '2':
      return std::wstring(1, shiftDown ? L'@' : L'2');
    case '3':
      return std::wstring(1, shiftDown ? L'#' : L'3');
    case '4':
      return std::wstring(1, shiftDown ? L'$' : L'4');
    case '5':
      return std::wstring(1, shiftDown ? L'%' : L'5');
    case '6':
      return std::wstring(1, shiftDown ? L'^' : L'6');
    case '7':
      return std::wstring(1, shiftDown ? L'&' : L'7');
    case '8':
      return std::wstring(1, shiftDown ? L'*' : L'8');
    case '9':
      return std::wstring(1, shiftDown ? L'(' : L'9');
    case VK_NUMPAD0:
      return L"0";
    case VK_NUMPAD1:
      return L"1";
    case VK_NUMPAD2:
      return L"2";
    case VK_NUMPAD3:
      return L"3";
    case VK_NUMPAD4:
      return L"4";
    case VK_NUMPAD5:
      return L"5";
    case VK_NUMPAD6:
      return L"6";
    case VK_NUMPAD7:
      return L"7";
    case VK_NUMPAD8:
      return L"8";
    case VK_NUMPAD9:
      return L"9";
    case VK_ADD:
      return L"+";
    case VK_SUBTRACT:
      return L"-";
    case VK_MULTIPLY:
      return L"*";
    case VK_DIVIDE:
      return L"/";
    case VK_DECIMAL:
    case VK_SEPARATOR:
      return L".";
    case VK_OEM_1:
      return std::wstring(1, shiftDown ? L':' : L';');
    case VK_OEM_2:
      return std::wstring(1, shiftDown ? L'?' : L'/');
    case VK_OEM_3:
      return std::wstring(1, shiftDown ? L'~' : L'`');
    case VK_OEM_4:
      return std::wstring(1, shiftDown ? L'{' : L'[');
    case VK_OEM_5:
      return std::wstring(1, shiftDown ? L'|' : L'\\');
    case VK_OEM_6:
      return std::wstring(1, shiftDown ? L'}' : L']');
    case VK_OEM_7:
      return std::wstring(1, shiftDown ? L'"' : L'\'');
    case VK_OEM_PLUS:
      return std::wstring(1, shiftDown ? L'+' : L'=');
    case VK_OEM_COMMA:
      return std::wstring(1, shiftDown ? L'<' : L',');
    case VK_OEM_MINUS:
      return std::wstring(1, shiftDown ? L'_' : L'-');
    case VK_OEM_PERIOD:
      return std::wstring(1, shiftDown ? L'>' : L'.');
    default:
      return {};
  }
}

TF_DA_COLOR NoColor() {
  TF_DA_COLOR color = {};
  color.type = TF_CT_NONE;
  color.cr = 0;
  return color;
}

TF_DA_COLOR RgbColor(COLORREF colorValue) {
  TF_DA_COLOR color = {};
  color.type = TF_CT_COLORREF;
  color.cr = colorValue;
  return color;
}

TF_DISPLAYATTRIBUTE DefaultDisplayAttribute() {
  TF_DISPLAYATTRIBUTE attr = {};
  attr.crText = NoColor();
  attr.crBk = NoColor();
  attr.lsStyle = TF_LS_SOLID;
  attr.fBoldLine = FALSE;
  attr.crLine = RgbColor(RGB(0, 120, 215));
  attr.bAttr = TF_ATTR_INPUT;
  return attr;
}

bool IsUsableRect(const RECT& rect) {
  if (rect.bottom <= rect.top) return false;
  if (rect.right < rect.left) return false;
  if (rect.left == 0 && rect.right == 0 && rect.top == 0 && rect.bottom == 0) return false;
  RECT probe = rect;
  if (probe.right == probe.left) probe.right += 1;
  if (!MonitorFromRect(&probe, MONITOR_DEFAULTTONULL)) return false;
  return true;
}

HWND RootWindowForPlacement(HWND hwnd) {
  if (!hwnd) return nullptr;
  HWND root = GetAncestor(hwnd, GA_ROOT);
  return root ? root : hwnd;
}

bool GetUsableWindowRect(HWND hwnd, RECT* rect) {
  if (!hwnd || !rect || !IsWindowVisible(hwnd)) return false;
  HWND root = RootWindowForPlacement(hwnd);
  if (root && IsWindowVisible(root)) hwnd = root;

  HRESULT hr = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, rect, sizeof(*rect));
  if (FAILED(hr) || !IsUsableRect(*rect)) {
    if (!GetWindowRect(hwnd, rect) || !IsUsableRect(*rect)) return false;
  }
  return true;
}

bool PointInWindowForPlacement(HWND hwnd, POINT pt) {
  RECT rect = {};
  return GetUsableWindowRect(hwnd, &rect) && PtInRect(&rect, pt);
}

HWND ResolveContextWindow(ITfContext* context) {
  if (context) {
    ITfContextView* view = nullptr;
    if (SUCCEEDED(context->GetActiveView(&view)) && view) {
      HWND hwnd = nullptr;
      if (SUCCEEDED(view->GetWnd(&hwnd)) && hwnd) {
        view->Release();
        return hwnd;
      }
      view->Release();
    }
  }

  HWND hwnd = GetFocus();
  if (hwnd) return hwnd;
  return GetForegroundWindow();
}

bool TryGetTextExtRect(TfEditCookie ec, ITfContext* context, ITfRange* range, RECT* rect) {
  if (!context || !range || !rect) return false;

  ITfContextView* view = nullptr;
  if (FAILED(context->GetActiveView(&view)) || !view) return false;

  BOOL clipped = FALSE;
  const HRESULT hr = view->GetTextExt(ec, range, rect, &clipped);
  view->Release();
  return SUCCEEDED(hr) && !clipped && IsUsableRect(*rect);
}

bool TryGetGuiCaretRect(RECT* rect) {
  if (!rect) return false;
  GUITHREADINFO info = {};
  info.cbSize = sizeof(info);
  if (!GetGUIThreadInfo(0, &info)) return false;

  if (info.hwndCaret) {
    *rect = info.rcCaret;
    MapWindowPoints(info.hwndCaret, nullptr, reinterpret_cast<POINT*>(rect), 2);
    if (IsUsableRect(*rect)) return true;
  }

  POINT pt = {};
  if (!GetCaretPos(&pt)) return false;

  HWND hwnd = info.hwndFocus ? info.hwndFocus : GetForegroundWindow();
  if (hwnd) ClientToScreen(hwnd, &pt);

  rect->left = pt.x;
  rect->right = pt.x + 1;
  rect->top = pt.y;
  rect->bottom = pt.y + 20;
  return IsUsableRect(*rect);
}

bool TryGetMouseRect(HWND targetHwnd, RECT* rect) {
  if (!rect) return false;
  POINT pt = {};
  if (!GetCursorPos(&pt)) return false;
  if (!PointInWindowForPlacement(targetHwnd, pt)) return false;
  rect->left = pt.x;
  rect->right = pt.x + 1;
  rect->top = pt.y;
  rect->bottom = pt.y + 20;
  return IsUsableRect(*rect);
}

bool TryGetWindowBottomLeftRect(HWND hwnd, RECT* rect) {
  if (!hwnd || !rect || !IsWindowVisible(hwnd)) return false;
  RECT windowRect = {};
  if (!GetUsableWindowRect(hwnd, &windowRect)) return false;

  HMONITOR monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
  MONITORINFO mi = {};
  mi.cbSize = sizeof(mi);
  if (!monitor || !GetMonitorInfoW(monitor, &mi)) return false;

  const int margin = 24;
  const int x = std::clamp(static_cast<int>(windowRect.left + margin),
                           static_cast<int>(mi.rcWork.left + margin),
                           static_cast<int>(std::max<LONG>(mi.rcWork.left + margin, mi.rcWork.right - margin)));
  const int y = std::clamp(static_cast<int>(windowRect.bottom - margin),
                           static_cast<int>(mi.rcWork.top + margin),
                           static_cast<int>(std::max<LONG>(mi.rcWork.top + margin, mi.rcWork.bottom - margin)));
  rect->left = x;
  rect->right = x + 1;
  rect->top = y;
  rect->bottom = y + 20;
  return IsUsableRect(*rect);
}

bool TryGetScreenSafeRect(HWND targetHwnd, RECT* rect) {
  if (!rect) return false;
  HMONITOR monitor = nullptr;
  if (targetHwnd) monitor = MonitorFromWindow(targetHwnd, MONITOR_DEFAULTTONEAREST);
  if (!monitor) {
    HWND foreground = GetForegroundWindow();
    if (foreground) monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST);
  }
  if (!monitor) {
    POINT pt = {};
    if (!GetCursorPos(&pt)) {
      pt.x = 0;
      pt.y = 0;
    }
    monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
  }
  MONITORINFO mi = {};
  mi.cbSize = sizeof(mi);
  if (!monitor || !GetMonitorInfoW(monitor, &mi)) return false;
  rect->left = mi.rcWork.left + 24;
  rect->right = rect->left + 1;
  rect->top = mi.rcWork.top + 24;
  rect->bottom = rect->top + 20;
  return IsUsableRect(*rect);
}

bool TryGetFullscreenOverlayRect(HWND targetHwnd, RECT* rect) {
  if (!rect) return false;
  HMONITOR monitor = nullptr;
  if (targetHwnd) monitor = MonitorFromWindow(targetHwnd, MONITOR_DEFAULTTONEAREST);
  if (!monitor) {
    HWND foreground = GetForegroundWindow();
    if (foreground) monitor = MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST);
  }
  if (!monitor) return TryGetScreenSafeRect(targetHwnd, rect);

  MONITORINFO mi = {};
  mi.cbSize = sizeof(mi);
  if (!GetMonitorInfoW(monitor, &mi)) return false;

  UINT dpi = DpiForScreenRect(&mi.rcMonitor);
  if (dpi == 0) dpi = 96;
  const int margin = MulDiv(48, static_cast<int>(dpi), 96);
  const int height = MulDiv(20, static_cast<int>(dpi), 96);
  const LONG left = mi.rcMonitor.left + margin;
  const LONG bottom = mi.rcMonitor.bottom - margin;
  rect->left = left;
  rect->right = left + 1;
  rect->bottom = bottom;
  rect->top = bottom - std::max(1, height);
  return IsUsableRect(*rect);
}

void DebugLogCandidateAnchorSource(const wchar_t* source, const RECT& rect) {
  if (!SrfTsfDebugTraceEnabled()) return;
  wchar_t buf[192] = {};
  swprintf_s(buf, L"CandidateAnchor source=%s rect=(%ld,%ld,%ld,%ld)", source, rect.left,
             rect.top, rect.right, rect.bottom);
  SrfTsfDebugLog(buf);
}

std::wstring BaseName(std::wstring path) {
  const size_t slash = path.find_last_of(L"\\/");
  if (slash != std::wstring::npos) path = path.substr(slash + 1);
  return path;
}

std::wstring LowerWide(std::wstring value) {
  std::transform(value.begin(), value.end(), value.begin(),
                 [](wchar_t ch) { return static_cast<wchar_t>(towlower(ch)); });
  return value;
}

bool WildcardMatchNoCase(const std::wstring& pattern, const std::wstring& value) {
  const std::wstring pat = LowerWide(pattern);
  const std::wstring text = LowerWide(value);
  size_t p = 0;
  size_t t = 0;
  size_t star = std::wstring::npos;
  size_t retry = 0;
  while (t < text.size()) {
    if (p < pat.size() && (pat[p] == L'?' || pat[p] == text[t])) {
      ++p;
      ++t;
    } else if (p < pat.size() && pat[p] == L'*') {
      star = p++;
      retry = t;
    } else if (star != std::wstring::npos) {
      p = star + 1;
      t = ++retry;
    } else {
      return false;
    }
  }
  while (p < pat.size() && pat[p] == L'*') ++p;
  return p == pat.size();
}

bool IsBuiltinStrictFocusProcessName(const std::wstring& appName) {
  if (appName.empty()) return false;
  const std::wstring baseName = BaseName(appName);
  const wchar_t* patterns[] = {
      L"winword.exe",
      L"excel.exe",
      L"powerpnt.exe",
      L"outlook.exe",
      L"onenote.exe",
      L"wps.exe",
      L"wpp.exe",
      L"et.exe",
      L"chrome.exe",
      L"msedge.exe",
      L"firefox.exe",
      L"applicationframehost.exe",
      L"wechat.exe",
      L"wechatapp.exe",
      L"qq.exe",
      L"dingding.exe",
      L"feishu.exe",
      L"lark.exe",
      L"slack.exe",
      L"teams.exe",
      L"code.exe",
      L"cursor.exe",
  };
  for (const wchar_t* pattern : patterns) {
    if (WildcardMatchNoCase(pattern, baseName)) return true;
  }
  return false;
}

std::wstring WindowClassName(HWND hwnd) {
  wchar_t name[128] = {};
  if (!hwnd || GetClassNameW(hwnd, name, static_cast<int>(_countof(name))) <= 0) return {};
  return name;
}

std::wstring SanitizeDiagnosticValue(std::wstring value, size_t maxUnits = 64) {
  if (value.empty()) return L"(none)";
  for (wchar_t& ch : value) {
    if (ch == L' ' || ch == L',' || ch == L';' || ch == L'=' || ch == L'\r' || ch == L'\n' ||
        ch == L'\t') {
      ch = L'_';
    }
  }
  if (value.size() > maxUnits) {
    value.resize(maxUnits);
  }
  return value;
}

bool CurrentProcessAppContainerState(bool* known) {
  if (known) *known = false;
  HANDLE token = nullptr;
  if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) return false;

  DWORD value = 0;
  DWORD bytes = sizeof(value);
  const BOOL ok =
      GetTokenInformation(token, TokenIsAppContainer, &value, sizeof(value), &bytes);
  CloseHandle(token);
  if (!ok) return false;
  if (known) *known = true;
  return value != 0;
}

std::wstring CurrentProcessIntegrityLabel() {
  HANDLE token = nullptr;
  if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token)) return L"unknown";

  DWORD bytes = 0;
  (void)GetTokenInformation(token, TokenIntegrityLevel, nullptr, 0, &bytes);
  if (bytes == 0) {
    CloseHandle(token);
    return L"unknown";
  }

  std::vector<BYTE> buffer(bytes);
  if (!GetTokenInformation(token, TokenIntegrityLevel, buffer.data(), bytes, &bytes)) {
    CloseHandle(token);
    return L"unknown";
  }
  CloseHandle(token);

  const auto* label = reinterpret_cast<const TOKEN_MANDATORY_LABEL*>(buffer.data());
  if (!label || !label->Label.Sid) return L"unknown";
  const UCHAR subAuthCount = *GetSidSubAuthorityCount(label->Label.Sid);
  if (subAuthCount == 0) return L"unknown";
  const DWORD rid = *GetSidSubAuthority(label->Label.Sid, subAuthCount - 1);
  if (rid < SECURITY_MANDATORY_MEDIUM_RID) return L"low";
  if (rid < SECURITY_MANDATORY_HIGH_RID) return L"medium";
  if (rid < SECURITY_MANDATORY_SYSTEM_RID) return L"high";
  return L"system";
}

bool IsBuiltinGameWindowClass(const std::wstring& className) {
  if (className.empty()) return false;
  const wchar_t* classes[] = {
      L"UnityWndClass",
      L"UnrealWindow",
      L"SDL_app",
      L"GLFW30",
      L"Valve001",
      L"LaunchUnrealUWindowsClient",
  };
  for (const wchar_t* cls : classes) {
    if (_wcsicmp(className.c_str(), cls) == 0) return true;
  }
  return false;
}

bool IsBuiltinGameProcessName(const std::wstring& appName) {
  if (appName.empty()) return false;
  const wchar_t* patterns[] = {
      L"*-win64-shipping.exe",
      L"*-win32-shipping.exe",
      L"cs2.exe",
      L"dota2.exe",
      L"valorant-win64-shipping.exe",
      L"fortniteclient-win64-shipping.exe",
      L"league of legends.exe",
      L"eldenring.exe",
      L"genshinimpact.exe",
      L"yuanshen.exe",
      L"starrail.exe",
      L"zenlesszonezero.exe",
      L"minecraft*.exe",
      L"r5apex.exe",
      L"overwatch.exe",
      L"wow.exe",
      L"ffxiv_dx11.exe",
      L"ffxiv.exe",
      L"ffxivboot.exe",
      L"ffxivlauncher.exe",
      L"destiny2.exe",
      L"helldivers2.exe",
      L"cyberpunk2077.exe",
      L"witcher3.exe",
      L"blackmythwukong-win64-shipping.exe",
      L"palworld-win64-shipping.exe",
      L"hogwartslegacy.exe",
      L"starfield.exe",
      L"forzahorizon*.exe",
      L"forzamotorsport.exe",
      L"cod.exe",
      L"cod22-cod.exe",
      L"modernwarfare*.exe",
      L"bf*.exe",
      L"titanfall2.exe",
      L"pubg*.exe",
      L"tarkov.exe",
      L"escapefromtarkov.exe",
      L"rainbowsix.exe",
      L"rainbowsix_vulkan.exe",
      L"deadbydaylight-win64-shipping.exe",
      L"left4dead2.exe",
      L"gta5.exe",
      L"rdr2.exe",
      L"warframe.x64.exe",
      L"pathofexile*.exe",
      L"poe*.exe",
      L"diablo iv.exe",
      L"diabloiii64.exe",
      L"sekiro.exe",
      L"armoredcore6.exe",
      L"monsterhunterworld.exe",
      L"monsterhunterwilds.exe",
      L"re4.exe",
      L"re8.exe",
      L"re2.exe",
      L"re3.exe",
      L"tekken8.exe",
      L"streetfighter6.exe",
      L"nba2k*.exe",
      L"fc*.exe",
      L"eafc*.exe",
  };
  const std::wstring baseName = BaseName(appName);
  for (const wchar_t* pattern : patterns) {
    if (WildcardMatchNoCase(pattern, baseName)) return true;
  }
  return false;
}

bool IsBuiltinAsciiOnlyProcessName(const std::wstring& appName) {
  if (appName.empty()) return false;
  const std::wstring baseName = BaseName(appName);
  const wchar_t* patterns[] = {
      L"transerr.exe",  // WPS crash reporter; keep TIP passive inside the reporter.
  };
  for (const wchar_t* pattern : patterns) {
    if (WildcardMatchNoCase(pattern, baseName)) return true;
  }
  return false;
}

bool IsConfiguredGameProcessName(const SrfConfig& config, const std::wstring& appName) {
  if (appName.empty()) return false;
  const std::wstring baseName = BaseName(appName);
  for (const auto& pattern : config.compatibility.gameProcessList) {
    if (!pattern.empty() &&
        (WildcardMatchNoCase(pattern, appName) || WildcardMatchNoCase(pattern, baseName))) {
      return true;
    }
  }
  return false;
}

bool MatchesConfiguredProcessList(const std::vector<std::wstring>& patterns,
                                  const std::wstring& appName) {
  if (appName.empty()) return false;
  const std::wstring baseName = BaseName(appName);
  for (const auto& pattern : patterns) {
    if (!pattern.empty() &&
        (WildcardMatchNoCase(pattern, appName) || WildcardMatchNoCase(pattern, baseName))) {
      return true;
    }
  }
  return false;
}

bool IsFullscreenForegroundWindow(HWND hwnd) {
  if (!hwnd || !IsWindowVisible(hwnd)) return false;

  HWND root = GetAncestor(hwnd, GA_ROOT);
  if (root && IsWindowVisible(root)) hwnd = root;

  const LONG_PTR exStyle = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
  if ((exStyle & WS_EX_TOOLWINDOW) != 0) return false;

  RECT windowRect = {};
  HRESULT hr = DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &windowRect,
                                     sizeof(windowRect));
  if (FAILED(hr) || !IsUsableRect(windowRect)) {
    if (!GetWindowRect(hwnd, &windowRect) || !IsUsableRect(windowRect)) return false;
  }

  HMONITOR monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
  MONITORINFO mi = {};
  mi.cbSize = sizeof(mi);
  if (!monitor || !GetMonitorInfoW(monitor, &mi)) return false;

  const RECT monitorRect = mi.rcMonitor;
  const LONG_PTR style = GetWindowLongPtrW(hwnd, GWL_STYLE);
  const bool popupWindow = (style & WS_POPUP) != 0;
  const bool captionlessWindow = (style & WS_CAPTION) == 0 && (style & WS_THICKFRAME) == 0;
  const bool borderless = popupWindow || captionlessWindow;
  const bool knownGameClass = IsBuiltinGameWindowClass(WindowClassName(hwnd));

  const bool coversMonitor =
      CandidateOverlayRectCoversMonitor(windowRect, monitorRect);
  HWND owner = GetWindow(hwnd, GW_OWNER);
  if (owner && IsWindowVisible(owner)) return false;
  if (!coversMonitor) return false;

  int score = 3;
  if (popupWindow) score += 2;
  if (captionlessWindow) score += 2;
  if (borderless) score += 1;
  if (knownGameClass) score += 2;
  return score >= 5;
}

std::filesystem::path LocalDataDir() {
  wchar_t localAppData[MAX_PATH] = {};
  const DWORD len = GetEnvironmentVariableW(L"LOCALAPPDATA", localAppData, MAX_PATH);
  if (len == 0 || len >= MAX_PATH) return {};
  return std::filesystem::path(localAppData) / L"kaixin";
}

void AppendUtf8FileLine(const std::filesystem::path& path, const std::wstring& line) {
  if (path.empty()) return;
  std::error_code ec;
  std::filesystem::create_directories(path.parent_path(), ec);

  const int bytes = WideCharToMultiByte(CP_UTF8, 0, line.c_str(), static_cast<int>(line.size()),
                                        nullptr, 0, nullptr, nullptr);
  if (bytes <= 0) return;
  std::string utf8(static_cast<size_t>(bytes), '\0');
  WideCharToMultiByte(CP_UTF8, 0, line.c_str(), static_cast<int>(line.size()), utf8.data(),
                      bytes, nullptr, nullptr);
  utf8.append("\r\n");

  HANDLE file = CreateFileW(path.c_str(), FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE,
                            nullptr, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) return;
  DWORD written = 0;
  WriteFile(file, utf8.data(), static_cast<DWORD>(utf8.size()), &written, nullptr);
  CloseHandle(file);
}

std::filesystem::path ModuleDirFromAddress(const void* address) {
  HMODULE module = nullptr;
  if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
                              GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                          reinterpret_cast<LPCWSTR>(address), &module) ||
      !module) {
    return {};
  }

  wchar_t path[MAX_PATH] = {};
  if (!GetModuleFileNameW(module, path, MAX_PATH)) return {};
  return std::filesystem::path(path).parent_path();
}

std::filesystem::path FindTrayHelper(const std::filesystem::path& moduleDir) {
  std::error_code ec;
  std::filesystem::path current = moduleDir;
  for (int depth = 0; depth < 5 && !current.empty(); ++depth) {
    const auto candidate = current / L"srf_ime_tray.exe";
    if (!candidate.empty() && std::filesystem::is_regular_file(candidate, ec)) return candidate;
    ec.clear();
    current = current.parent_path();
  }
  return {};
}

std::filesystem::path FindSettingsHelper(const std::filesystem::path& moduleDir) {
  std::error_code ec;
  std::filesystem::path current = moduleDir;
  for (int depth = 0; depth < 5 && !current.empty(); ++depth) {
    const auto candidate = current / L"srf_ime_settings.exe";
    if (!candidate.empty() && std::filesystem::is_regular_file(candidate, ec)) return candidate;
    ec.clear();
    current = current.parent_path();
  }
  return {};
}

HRESULT LaunchSettingsHelper(HWND parent) {
  const auto settingsExe = FindSettingsHelper(ModuleDirFromAddress(&LaunchSettingsHelper));
  if (settingsExe.empty()) return HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND);

  std::wstring commandLine = L"\"" + settingsExe.wstring() + L"\"";
  std::wstring workDir = settingsExe.parent_path().wstring();

  STARTUPINFOW startup = {};
  startup.cb = sizeof(startup);
  PROCESS_INFORMATION processInfo = {};
  if (parent) {
    startup.dwFlags = STARTF_USESHOWWINDOW;
    startup.wShowWindow = SW_SHOWNORMAL;
  }
  if (!CreateProcessW(settingsExe.c_str(), commandLine.data(), nullptr, nullptr, FALSE,
                      CREATE_DEFAULT_ERROR_MODE, nullptr,
                      workDir.empty() ? nullptr : workDir.c_str(), &startup, &processInfo)) {
    return HRESULT_FROM_WIN32(GetLastError());
  }
  CloseHandle(processInfo.hThread);
  CloseHandle(processInfo.hProcess);
  return S_OK;
}

void EnsureTrayHelperRunning() {
  if (InstallMaintenanceActive()) return;

  HANDLE mutex = OpenMutexW(SYNCHRONIZE, FALSE, kTrayMutexName);
  if (mutex) {
    CloseHandle(mutex);
    return;
  }

  const auto trayExe = FindTrayHelper(ModuleDirFromAddress(&EnsureTrayHelperRunning));
  if (trayExe.empty()) return;

  std::wstring commandLine = L"\"" + trayExe.wstring() + L"\"";
  std::wstring workDir = trayExe.parent_path().wstring();

  STARTUPINFOW startup = {};
  startup.cb = sizeof(startup);
  PROCESS_INFORMATION processInfo = {};
  if (CreateProcessW(trayExe.c_str(), commandLine.data(), nullptr, nullptr, FALSE,
                     CREATE_DEFAULT_ERROR_MODE, nullptr,
                     workDir.empty() ? nullptr : workDir.c_str(), &startup, &processInfo)) {
    CloseHandle(processInfo.hThread);
    CloseHandle(processInfo.hProcess);
  }
}

UINT PageStartFromMetrics(const CandidatePageLayoutMetrics& metrics, UINT page) {
  if (metrics.pageStarts.empty()) return 0;
  const UINT clampedPage =
      std::min(page, static_cast<UINT>(metrics.pageStarts.size() - 1));
  return metrics.pageStarts[clampedPage];
}

UINT PageEndExclusiveFromMetrics(const CandidatePageLayoutMetrics& metrics, UINT page,
                                 UINT totalItems) {
  if (metrics.pageStarts.empty()) return totalItems;
  const UINT clampedPage =
      std::min(page, static_cast<UINT>(metrics.pageStarts.size() - 1));
  if (clampedPage + 1 < metrics.pageStarts.size()) return metrics.pageStarts[clampedPage + 1];
  return totalItems;
}

UINT PageForIndexFromMetrics(const CandidatePageLayoutMetrics& metrics, UINT index, UINT totalItems) {
  if (metrics.pageStarts.empty() || totalItems == 0) return 0;
  const UINT clampedIndex = std::min(index, totalItems - 1);
  auto it = std::upper_bound(metrics.pageStarts.begin(), metrics.pageStarts.end(), clampedIndex);
  if (it == metrics.pageStarts.begin()) return 0;
  return static_cast<UINT>((it - metrics.pageStarts.begin()) - 1);
}

HRESULT GetInsertionRange(TfEditCookie ec, ITfContext* pic, ITfRange** ppRange) {
  if (!ppRange) return E_POINTER;
  *ppRange = nullptr;

  TF_SELECTION sel = {};
  ULONG count = 0;
  HRESULT hr = pic->GetSelection(ec, TF_DEFAULT_SELECTION, 1, &sel, &count);
  if (SUCCEEDED(hr) && count > 0 && sel.range) {
    *ppRange = sel.range;
    return S_OK;
  }

  ITfInsertAtSelection* insertAtSelection = nullptr;
  hr = pic->QueryInterface(IID_ITfInsertAtSelection, reinterpret_cast<void**>(&insertAtSelection));
  if (FAILED(hr)) return hr;

  ITfRange* range = nullptr;
  hr = insertAtSelection->InsertTextAtSelection(ec, TF_IAS_QUERYONLY, nullptr, 0, &range);
  insertAtSelection->Release();
  if (FAILED(hr)) return hr;

  *ppRange = range;
  return S_OK;
}

HRESULT SetCompositionDisplay(TfEditCookie ec, ITfComposition* composition, const std::wstring& text) {
  if (!composition) return E_FAIL;
  ITfRange* range = nullptr;
  HRESULT hr = composition->GetRange(&range);
  if (FAILED(hr) || !range) return FAILED(hr) ? hr : E_FAIL;
  hr = range->SetText(ec, 0, text.c_str(), static_cast<LONG>(text.size()));
  range->Release();
  return hr;
}

HRESULT CollapseSelectionToRangeOffset(TfEditCookie ec, ITfContext* pic, ITfRange* range,
                                       LONG offset) {
  if (!pic || !range || offset < 0) return E_INVALIDARG;

  ITfRange* selectionRange = nullptr;
  HRESULT hr = range->Clone(&selectionRange);
  if (FAILED(hr) || !selectionRange) return FAILED(hr) ? hr : E_FAIL;

  hr = selectionRange->Collapse(ec, TF_ANCHOR_START);
  if (SUCCEEDED(hr) && offset > 0) {
    LONG shifted = 0;
    hr = selectionRange->ShiftEnd(ec, offset, &shifted, nullptr);
    if (SUCCEEDED(hr)) {
      hr = selectionRange->Collapse(ec, TF_ANCHOR_END);
    }
  }
  if (SUCCEEDED(hr)) {
    TF_SELECTION selection = {};
    selection.range = selectionRange;
    selection.style.ase = TF_AE_NONE;
    selection.style.fInterimChar = FALSE;
    hr = pic->SetSelection(ec, 1, &selection);
  }

  selectionRange->Release();
  return hr;
}

HRESULT CollapseSelectionToRangeEnd(TfEditCookie ec, ITfContext* pic, ITfRange* range) {
  if (!pic || !range) return E_INVALIDARG;

  ITfRange* selectionRange = nullptr;
  HRESULT hr = range->Clone(&selectionRange);
  if (FAILED(hr) || !selectionRange) return FAILED(hr) ? hr : E_FAIL;

  hr = selectionRange->Collapse(ec, TF_ANCHOR_END);
  if (SUCCEEDED(hr)) {
    TF_SELECTION selection = {};
    selection.range = selectionRange;
    selection.style.ase = TF_AE_NONE;
    selection.style.fInterimChar = FALSE;
    hr = pic->SetSelection(ec, 1, &selection);
  }

  selectionRange->Release();
  return hr;
}

HRESULT DeleteSelectionOrPreviousText(TfEditCookie ec, ITfContext* pic) {
  if (!pic) return E_INVALIDARG;

  ITfRange* range = nullptr;
  HRESULT hr = GetInsertionRange(ec, pic, &range);
  if (FAILED(hr) || !range) return FAILED(hr) ? hr : S_FALSE;
  BOOL isEmpty = TRUE;
  hr = range->IsEmpty(ec, &isEmpty);
  if (FAILED(hr)) {
    range->Release();
    return hr;
  }

  if (!isEmpty) {
    hr = range->SetText(ec, 0, L"", 0);
    range->Release();
    return hr;
  }

  ITfRange* deleteRange = nullptr;
  hr = range->Clone(&deleteRange);
  range->Release();
  if (FAILED(hr) || !deleteRange) return FAILED(hr) ? hr : E_FAIL;

  LONG shifted = 0;
  hr = deleteRange->ShiftStart(ec, -1, &shifted, nullptr);
  if (FAILED(hr) || shifted == 0) {
    deleteRange->Release();
    return FAILED(hr) ? hr : S_FALSE;
  }

  hr = deleteRange->SetText(ec, 0, L"", 0);
  deleteRange->Release();
  return hr;
}

class CEditSessionCancelFocus final : public ITfEditSession {
  LONG m_cRef = 1;
  CSrfTip* m_tip = nullptr;
  uint64_t m_generation = 0;
  uint64_t m_cancelSequence = 0;

 public:
  CEditSessionCancelFocus(CSrfTip* tip, uint64_t generation, uint64_t cancelSequence)
      : m_tip(tip), m_generation(generation), m_cancelSequence(cancelSequence) {
    if (m_tip) m_tip->AddRef();
  }

  ~CEditSessionCancelFocus() {
    if (m_tip) m_tip->Release();
  }

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (riid == IID_IUnknown || riid == IID_ITfEditSession) {
      *ppv = static_cast<ITfEditSession*>(this);
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }

  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_cRef); }

  STDMETHODIMP_(ULONG) Release() override {
    const ULONG count = InterlockedDecrement(&m_cRef);
    if (count == 0) delete this;
    return count;
  }

  STDMETHODIMP DoEditSession(TfEditCookie ec) override {
    if (!m_tip) return E_FAIL;
    m_tip->HandleFocusLossCancelEditSession(ec, m_generation, m_cancelSequence);
    return S_OK;
  }
};

class CEditSessionCompatibilityAsciiCleanup final : public ITfEditSession {
  LONG m_cRef = 1;
  CSrfTip* m_tip = nullptr;

 public:
  explicit CEditSessionCompatibilityAsciiCleanup(CSrfTip* tip) : m_tip(tip) {
    if (m_tip) m_tip->AddRef();
  }

  ~CEditSessionCompatibilityAsciiCleanup() {
    if (m_tip) m_tip->Release();
  }

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (riid == IID_IUnknown || riid == IID_ITfEditSession) {
      *ppv = static_cast<ITfEditSession*>(this);
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }

  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_cRef); }

  STDMETHODIMP_(ULONG) Release() override {
    const ULONG count = InterlockedDecrement(&m_cRef);
    if (count == 0) delete this;
    return count;
  }

  STDMETHODIMP DoEditSession(TfEditCookie ec) override {
    if (!m_tip) return E_FAIL;
    m_tip->HandleCompatibilityAsciiCleanupEditSession(ec);
    return S_OK;
  }
};

class CEditSessionCommitCandidate final : public ITfEditSession {
  LONG m_cRef = 1;
  CSrfTip* m_tip = nullptr;
  ITfContext* m_context = nullptr;
  size_t m_index = 0;
  std::wstring m_reading;
  std::wstring m_committed;
  std::wstring m_meta;
  std::vector<std::wstring> m_skippedCandidates;

 public:
  CEditSessionCommitCandidate(CSrfTip* tip, ITfContext* context, size_t index,
                              std::wstring reading, std::wstring committed, std::wstring meta,
                              std::vector<std::wstring> skippedCandidates)
      : m_tip(tip),
        m_context(context),
        m_index(index),
        m_reading(std::move(reading)),
        m_committed(std::move(committed)),
        m_meta(std::move(meta)),
        m_skippedCandidates(std::move(skippedCandidates)) {
    if (m_tip) m_tip->AddRef();
    if (m_context) m_context->AddRef();
  }

  ~CEditSessionCommitCandidate() {
    if (m_context) m_context->Release();
    if (m_tip) m_tip->Release();
  }

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (riid == IID_IUnknown || riid == IID_ITfEditSession) {
      *ppv = static_cast<ITfEditSession*>(this);
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }

  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_cRef); }

  STDMETHODIMP_(ULONG) Release() override {
    const ULONG count = InterlockedDecrement(&m_cRef);
    if (count == 0) delete this;
    return count;
  }

  STDMETHODIMP DoEditSession(TfEditCookie ec) override {
    if (!m_tip) return E_FAIL;
    return m_tip->CommitCandidateSnapshot(ec, m_context, m_index, m_reading, m_committed, m_meta,
                                          m_skippedCandidates);
  }
};

class CEditSessionCommitReadingText final : public ITfEditSession {
  LONG m_cRef = 1;
  CSrfTip* m_tip = nullptr;

 public:
  explicit CEditSessionCommitReadingText(CSrfTip* tip) : m_tip(tip) {
    if (m_tip) m_tip->AddRef();
  }

  ~CEditSessionCommitReadingText() {
    if (m_tip) m_tip->Release();
  }

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (riid == IID_IUnknown || riid == IID_ITfEditSession) {
      *ppv = static_cast<ITfEditSession*>(this);
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }

  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_cRef); }

  STDMETHODIMP_(ULONG) Release() override {
    const ULONG count = InterlockedDecrement(&m_cRef);
    if (count == 0) delete this;
    return count;
  }

  STDMETHODIMP DoEditSession(TfEditCookie ec) override {
    if (!m_tip) return E_FAIL;
    return m_tip->CommitReadingText(ec, nullptr);
  }
};

class CSrfDisplayAttributeInfo final : public ITfDisplayAttributeInfo {
  LONG m_cRef = 1;
  TF_DISPLAYATTRIBUTE m_attr = DefaultDisplayAttribute();

 public:
  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (riid == IID_IUnknown || riid == IID_ITfDisplayAttributeInfo) {
      *ppv = static_cast<ITfDisplayAttributeInfo*>(this);
      AddRef();
      return S_OK;
    }
    return E_NOINTERFACE;
  }

  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_cRef); }

  STDMETHODIMP_(ULONG) Release() override {
    const ULONG count = InterlockedDecrement(&m_cRef);
    if (count == 0) delete this;
    return count;
  }

  STDMETHODIMP GetGUID(GUID* guid) override;
  STDMETHODIMP GetDescription(BSTR* description) override;
  STDMETHODIMP GetAttributeInfo(TF_DISPLAYATTRIBUTE* attr) override;
  STDMETHODIMP SetAttributeInfo(const TF_DISPLAYATTRIBUTE* attr) override;
  STDMETHODIMP Reset() override;
};

class CSrfEnumDisplayAttributeInfo final : public IEnumTfDisplayAttributeInfo {
  LONG m_cRef = 1;
  ULONG m_index = 0;

 public:
  explicit CSrfEnumDisplayAttributeInfo(ULONG index = 0) : m_index(index) {}

  STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override;
  STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_cRef); }
  STDMETHODIMP_(ULONG) Release() override;
  STDMETHODIMP Clone(IEnumTfDisplayAttributeInfo** ppEnum) override;
  STDMETHODIMP Next(ULONG count, ITfDisplayAttributeInfo** info, ULONG* fetched) override;
  STDMETHODIMP Reset() override;
  STDMETHODIMP Skip(ULONG count) override;
};

}  // namespace
