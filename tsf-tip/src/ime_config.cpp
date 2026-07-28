#include "ime_config.h"

#include "config_schema.h"

#include <windows.h>
#include <msctf.h>

#include <algorithm>
#include <array>
#include <atomic>
#include <cctype>
#include <cwctype>
#include <cstdint>
#include <fstream>
#include <iterator>
#include <limits>
#include <mutex>
#include <string>
#include <thread>
#include <unordered_map>
#include <utility>
#include <vector>

void LoadSkinFileFromConfigDir(SrfConfig& config, const std::filesystem::path& configDir);
extern "C" void SrfTip_BackgroundWorkerAddRef();
extern "C" void SrfTip_BackgroundWorkerRelease();

namespace {

constexpr wchar_t kConfigFileName[] = L"kaixin.ini";
constexpr wchar_t kLocalConfigSubdir[] = L"kaixin";
constexpr DWORD kConfigWatchFallbackIntervalMs = 30000;
constexpr DWORD kConfigWatchDebounceMs = 80;

struct ConfigCache {
  std::mutex mutex;
  SrfConfig config = {};
  std::filesystem::path path;
  std::filesystem::file_time_type writeTime = {};
  bool hasWriteTime = false;
  uint64_t version = 0;
  bool initialized = false;
};

struct IniSection {
  std::wstring name;
  std::wstring loweredName;
  std::unordered_map<std::wstring, std::wstring> values;
};

struct IniDocument {
  std::filesystem::path path;
  std::vector<IniSection> sections;
};

ConfigCache& GetConfigCache() {
  static ConfigCache cache;
  return cache;
}

std::atomic<uint64_t>& GetConfigVersionAtomic() {
  static std::atomic<uint64_t> version{0};
  return version;
}

std::wstring ToLower(std::wstring value) {
  std::transform(value.begin(), value.end(), value.begin(), towlower);
  return value;
}

std::wstring Trim(const std::wstring& value) {
  const auto first = value.find_first_not_of(L" \t\r\n");
  if (first == std::wstring::npos) return {};
  const auto last = value.find_last_not_of(L" \t\r\n");
  return value.substr(first, last - first + 1);
}

std::wstring StripBomThenTrim(std::wstring value) {
  if (!value.empty() && value.front() == L'\ufeff') value.erase(value.begin());
  return Trim(value);
}

std::wstring DecodeConfigBytes(const std::string& bytes) {
  if (bytes.empty()) return {};
  const char* data = bytes.data();
  int size = static_cast<int>(bytes.size());
  if (bytes.size() >= 3 && static_cast<unsigned char>(bytes[0]) == 0xEF &&
      static_cast<unsigned char>(bytes[1]) == 0xBB &&
      static_cast<unsigned char>(bytes[2]) == 0xBF) {
    data += 3;
    size -= 3;
  }

  UINT codePage = CP_UTF8;
  DWORD flags = MB_ERR_INVALID_CHARS;
  int needed = MultiByteToWideChar(codePage, flags, data, size, nullptr, 0);
  if (needed <= 0) {
    codePage = CP_ACP;
    flags = 0;
    needed = MultiByteToWideChar(codePage, flags, data, size, nullptr, 0);
  }
  if (needed <= 0) return {};

  std::wstring decoded(static_cast<size_t>(needed), L'\0');
  MultiByteToWideChar(codePage, flags, data, size, decoded.data(), needed);
  return decoded;
}

IniSection* FindOrAddIniSection(IniDocument& doc, const std::wstring& name) {
  const std::wstring lowered = ToLower(name);
  for (auto& section : doc.sections) {
    if (section.loweredName == lowered) return &section;
  }
  IniSection section;
  section.name = name;
  section.loweredName = lowered;
  doc.sections.push_back(std::move(section));
  return &doc.sections.back();
}

IniDocument ParseIniDocument(const std::filesystem::path& path, const std::wstring& text) {
  IniDocument doc;
  doc.path = path;
  IniSection* current = nullptr;
  size_t start = 0;
  while (start <= text.size()) {
    const size_t end = text.find_first_of(L"\r\n", start);
    std::wstring line =
        text.substr(start, end == std::wstring::npos ? std::wstring::npos : end - start);
    if (end == std::wstring::npos) {
      start = text.size() + 1;
    } else {
      start = end + 1;
      if (text[end] == L'\r' && start < text.size() && text[start] == L'\n') ++start;
    }

    const std::wstring trimmed = StripBomThenTrim(std::move(line));
    if (trimmed.empty() || trimmed[0] == L'#' || trimmed[0] == L';') continue;
    if (trimmed.front() == L'[' && trimmed.back() == L']' && trimmed.size() >= 2) {
      current = FindOrAddIniSection(doc, Trim(trimmed.substr(1, trimmed.size() - 2)));
      continue;
    }
    if (!current) continue;
    const size_t eq = trimmed.find(L'=');
    if (eq == std::wstring::npos) continue;
    const std::wstring key = ToLower(Trim(trimmed.substr(0, eq)));
    if (key.empty()) continue;
    current->values[key] = Trim(trimmed.substr(eq + 1));
  }
  return doc;
}

IniDocument LoadIniDocument(const std::filesystem::path& path) {
  std::ifstream file(path, std::ios::binary);
  if (!file) {
    IniDocument empty;
    empty.path = path;
    return empty;
  }
  std::string bytes((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
  return ParseIniDocument(path, DecodeConfigBytes(bytes));
}

thread_local const IniDocument* g_activeIniDocument = nullptr;

struct ActiveIniDocumentScope {
  explicit ActiveIniDocumentScope(const IniDocument& doc) : previous(g_activeIniDocument) {
    g_activeIniDocument = &doc;
  }
  ~ActiveIniDocumentScope() { g_activeIniDocument = previous; }
  const IniDocument* previous = nullptr;
};

const IniSection* FindIniSection(const IniDocument& doc, const wchar_t* section) {
  if (!section) return nullptr;
  const std::wstring lowered = ToLower(section);
  for (const auto& item : doc.sections) {
    if (item.loweredName == lowered) return &item;
  }
  return nullptr;
}

bool TryReadIniValue(const IniDocument& doc, const wchar_t* section, const wchar_t* key,
                     std::wstring* out) {
  if (!key || !out) return false;
  const IniSection* iniSection = FindIniSection(doc, section);
  if (!iniSection) return false;
  const std::wstring loweredKey = ToLower(key);
  auto it = iniSection->values.find(loweredKey);
  if (it == iniSection->values.end()) return false;
  *out = it->second;
  return true;
}

bool ActiveIniDocumentMatches(const std::filesystem::path& path) {
  return g_activeIniDocument && g_activeIniDocument->path == path;
}

bool ParseBool(const std::wstring& value, bool fallback) {
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered == L"1" || lowered == L"true" || lowered == L"yes" || lowered == L"on") return true;
  if (lowered == L"0" || lowered == L"false" || lowered == L"no" || lowered == L"off") return false;
  return fallback;
}

UINT ParseUInt(const std::wstring& value, UINT fallback) {
  const std::wstring trimmed = Trim(value);
  if (trimmed.empty()) return fallback;
  wchar_t* end = nullptr;
  const unsigned long parsed = wcstoul(trimmed.c_str(), &end, 10);
  if (!end || *end != L'\0') return fallback;
  return static_cast<UINT>(parsed);
}

bool TryParseInt(const std::wstring& value, int* output) {
  if (!output) return false;
  const std::wstring trimmed = Trim(value);
  if (trimmed.empty()) return false;
  wchar_t* end = nullptr;
  const long parsed = wcstol(trimmed.c_str(), &end, 10);
  if (!end || *end != L'\0' || parsed < (std::numeric_limits<int>::min)() ||
      parsed > (std::numeric_limits<int>::max)()) {
    return false;
  }
  *output = static_cast<int>(parsed);
  return true;
}

UINT HotkeyVkFromToken(const std::wstring& token) {
  if (token.size() == 1) {
    const wchar_t ch = towupper(token[0]);
    if ((ch >= L'A' && ch <= L'Z') || (ch >= L'0' && ch <= L'9')) return static_cast<UINT>(ch);
  }
  if (token.size() >= 2 && token[0] == L'f') {
    wchar_t* end = nullptr;
    const unsigned long parsed = wcstoul(token.c_str() + 1, &end, 10);
    if (end && *end == L'\0' && parsed >= 1 && parsed <= 24) {
      return VK_F1 + static_cast<UINT>(parsed - 1);
    }
  }
  if (token == L"space") return VK_SPACE;
  if (token == L"tab") return VK_TAB;
  if (token == L"enter" || token == L"return") return VK_RETURN;
  if (token == L"esc" || token == L"escape") return VK_ESCAPE;
  if (token == L"comma" || token == L",") return VK_OEM_COMMA;
  if (token == L"period" || token == L"dot" || token == L".") return VK_OEM_PERIOD;
  if (token == L"slash" || token == L"/") return VK_OEM_2;
  if (token == L"semicolon" || token == L";") return VK_OEM_1;
  if (token == L"quote" || token == L"apostrophe" || token == L"'") return VK_OEM_7;
  if (token == L"minus" || token == L"-") return VK_OEM_MINUS;
  if (token == L"equal" || token == L"equals" || token == L"=") return VK_OEM_PLUS;
  return 0;
}

bool TryParseHotkey(const std::wstring& value, UINT defaultVk, UINT defaultModifiers,
                    SrfHotkeyOptions* out) {
  if (!out) return false;
  const std::wstring lowered = ToLower(Trim(value));
  out->enabled = true;
  out->vk = defaultVk;
  out->modifiers = defaultModifiers;
  if (lowered.empty() || lowered == L"none" || lowered == L"disabled" || lowered == L"off" ||
      lowered == L"\u5173\u95ed") {
    out->enabled = false;
    return true;
  }

  UINT vk = 0;
  UINT modifiers = 0;
  bool sawKey = false;
  std::wstring token;
  std::wstring normalized = lowered;
  std::replace(normalized.begin(), normalized.end(), L'_', L'+');
  std::replace(normalized.begin(), normalized.end(), L'-', L'+');
  normalized.push_back(L'+');

  for (const wchar_t ch : normalized) {
    if (ch != L'+') {
      token.push_back(ch);
      continue;
    }
    token = Trim(token);
    if (token.empty()) continue;
    if (token == L"ctrl" || token == L"control") {
      modifiers |= TF_MOD_CONTROL;
    } else if (token == L"alt") {
      modifiers |= TF_MOD_ALT;
    } else if (token == L"shift") {
      modifiers |= TF_MOD_SHIFT;
    } else {
      if (sawKey) return true;
      vk = HotkeyVkFromToken(token);
      if (vk == 0) return true;
      sawKey = true;
    }
    token.clear();
  }
  if (!sawKey) vk = defaultVk;
  out->vk = vk;
  out->modifiers = modifiers == 0 ? defaultModifiers : modifiers;
  return true;
}

UINT ParseCnEnHotkey(const std::wstring& value, UINT fallback) {
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered == L"both" || lowered.empty()) return 0;
  if (lowered == L"ctrl_shift" || lowered == L"shift") return 1;
  if (lowered == L"ctrl_space") return 2;
  if (lowered == L"none" || lowered == L"disabled" || lowered == L"off") return 3;
  return fallback;
}

std::wstring ReadIniString(const std::filesystem::path& path, const wchar_t* section,
                           const wchar_t* key, const wchar_t* fallback = L"") {
  std::wstring value;
  if (ActiveIniDocumentMatches(path) &&
      TryReadIniValue(*g_activeIniDocument, section, key, &value)) {
    return value;
  }
  IniDocument doc = LoadIniDocument(path);
  return TryReadIniValue(doc, section, key, &value) ? value
                                                    : std::wstring(fallback ? fallback : L"");
}

// 候选字体可为绝对路径，需大于默认 ReadIniString 的 512 缓冲。
std::wstring ReadIniStringLong(const std::filesystem::path& path, const wchar_t* section,
                               const wchar_t* key, const wchar_t* fallback = L"") {
  return ReadIniString(path, section, key, fallback);
}

std::vector<std::wstring> ReadIniSectionNames(const std::filesystem::path& path) {
  IniDocument doc = ActiveIniDocumentMatches(path) ? *g_activeIniDocument : LoadIniDocument(path);
  std::vector<std::wstring> names;
  names.reserve(doc.sections.size());
  for (const auto& section : doc.sections) {
    names.push_back(section.name);
  }
  return names;
}

std::vector<std::wstring> ReadIniList(const std::filesystem::path& path, const wchar_t* section,
                                      const wchar_t* key) {
  const std::wstring raw = ReadIniString(path, section, key);
  std::vector<std::wstring> out;
  size_t start = 0;
  while (start <= raw.size()) {
    const size_t comma = raw.find(L',', start);
    std::wstring token =
        Trim(raw.substr(start, comma == std::wstring::npos ? std::wstring::npos : comma - start));
    if (!token.empty()) out.push_back(token);
    if (comma == std::wstring::npos) break;
    start = comma + 1;
  }
  return out;
}

void AppendDefaultGameProcessPatterns(std::vector<std::wstring>& list) {
  const wchar_t* defaults[] = {
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
      L"r5apex.exe",
      L"overwatch.exe",
      L"wow.exe",
      L"ffxiv_dx11.exe",
      L"destiny2.exe",
      L"helldivers2.exe",
      L"cyberpunk2077.exe",
      L"blackmythwukong-win64-shipping.exe",
      L"palworld-win64-shipping.exe",
      L"pubg*.exe",
      L"tarkov.exe",
      L"escapefromtarkov.exe",
      L"rainbowsix.exe",
      L"rainbowsix_vulkan.exe",
      L"gta5.exe",
      L"rdr2.exe",
      L"warframe.x64.exe",
      L"pathofexile*.exe",
      L"diablo iv.exe",
      L"sekiro.exe",
      L"armoredcore6.exe",
      L"monsterhunterworld.exe",
      L"tekken8.exe",
      L"streetfighter6.exe",
      L"*-win64-shipping.exe",
  };
  for (const wchar_t* value : defaults) {
    const bool exists = std::any_of(list.begin(), list.end(), [&](const std::wstring& item) {
      return _wcsicmp(item.c_str(), value) == 0;
    });
    if (!exists) list.push_back(value);
  }
}

std::filesystem::path ModuleDir() {
  HMODULE module = nullptr;
  if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS |
                              GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                          reinterpret_cast<LPCWSTR>(&ResolveSrfConfigPath), &module) ||
      !module) {
    return {};
  }

  wchar_t path[MAX_PATH] = {};
  if (!GetModuleFileNameW(module, path, MAX_PATH)) return {};
  return std::filesystem::path(path).parent_path();
}

std::filesystem::path LocalConfigPath() {
  wchar_t localAppData[MAX_PATH] = {};
  DWORD len = GetEnvironmentVariableW(L"LOCALAPPDATA", localAppData, MAX_PATH);
  if (len == 0 || len >= MAX_PATH) return {};
  return std::filesystem::path(localAppData) / kLocalConfigSubdir / kConfigFileName;
}

std::filesystem::path ModuleConfigPath() {
  return ModuleDir() / kConfigFileName;
}

std::filesystem::path ResolveSrfConfigPathInternal() {
  const std::filesystem::path local = LocalConfigPath();
  if (!local.empty() && std::filesystem::exists(local)) return local;

  const std::filesystem::path module = ModuleConfigPath();
  if (!module.empty() && std::filesystem::exists(module)) return module;

  return module;
}

// 皮肤文件/目录路径安全策略（避免在 IME 线程上访问不可达网络路径导致卡死）：
// - 拒绝 UNC/网络路径：\\server\share\... 以及 \\?\UNC\...
// - 绝对路径仅允许本地盘符（C:\... 或 \\?\C:\...）
// - 相对路径允许（将由上层解析到本地 roots）
bool IsAllowedLocalSkinPath(const std::filesystem::path& path) {
  if (path.empty()) return false;
  const std::wstring raw = path.wstring();
  if (raw.empty()) return false;

  // Win32 extended-length path prefix: \\?\...
  // 其中 \\?\UNC\... 仍是网络路径，必须拒绝。
  if (raw.rfind(L"\\\\?\\UNC\\", 0) == 0) return false;

  // 普通 UNC：\\server\share\...
  if (raw.rfind(L"\\\\", 0) == 0) return false;

  if (!path.is_absolute()) return true;

  // 允许 C:\... 的本地盘符形式。
  std::wstring root = path.root_name().wstring();
  if (root.size() >= 2 && root[1] == L':') return true;

  // 允许 \\?\C:\... 这类扩展本地盘符路径。
  if (raw.rfind(L"\\\\?\\", 0) == 0 && raw.size() >= 7 /* \\?\C:\ */) {
    const wchar_t drive = raw[4];
    const wchar_t colon = raw[5];
    const wchar_t slash = raw[6];
    if (((drive >= L'A' && drive <= L'Z') || (drive >= L'a' && drive <= L'z')) &&
        colon == L':' && (slash == L'\\' || slash == L'/')) {
      return true;
    }
  }

  return false;
}

SrfThemeMode ParseTheme(const std::filesystem::path& path) {
  const std::wstring v = ToLower(Trim(ReadIniString(path, SrfConfigSchema::section::kStyle,
                                                    SrfConfigSchema::key::kTheme,
                                                    SrfConfigSchema::defaults::kTheme)));
  if (v == L"light") return SrfThemeMode::Light;
  if (v == L"dark") return SrfThemeMode::Dark;
  if (v == L"high_contrast" || v == L"highcontrast" || v == L"hc") return SrfThemeMode::HighContrast;
  return SrfThemeMode::Auto;
}

SrfCandidateMaterial ParseCandidateMaterial(const std::wstring& value) {
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered == L"solid") return SrfCandidateMaterial::Solid;
  if (lowered == L"gradient") return SrfCandidateMaterial::Gradient;
  if (lowered == L"mist" || lowered == L"frosted" || lowered == L"acrylic") {
    return SrfCandidateMaterial::Mist;
  }
  return SrfCandidateMaterial::Auto;
}

SrfCandidateDensity ParseCandidateDensity(const std::wstring& value) {
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered == L"compact") return SrfCandidateDensity::Compact;
  if (lowered == L"comfortable" || lowered == L"comfort" || lowered == L"spacious") {
    return SrfCandidateDensity::Comfortable;
  }
  return SrfCandidateDensity::Standard;
}

SrfCandidateLayoutVariant ParseCandidateLayoutVariant(const std::wstring& value) {
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered == L"compact") return SrfCandidateLayoutVariant::Compact;
  if (lowered == L"card" || lowered == L"cards") return SrfCandidateLayoutVariant::Card;
  return SrfCandidateLayoutVariant::Classic;
}

SrfFullscreenPolicy ParseFullscreenPolicy(const std::wstring& value) {
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered == L"hide_ui" || lowered == L"hide-ui" || lowered == L"hideui" ||
      lowered == L"hide" || lowered == L"1") {
    return SrfFullscreenPolicy::HideUi;
  }
  if (lowered == L"off" || lowered == L"disabled" || lowered == L"disable" ||
      lowered == L"none" || lowered == L"0") {
    return SrfFullscreenPolicy::Off;
  }
  if (lowered == L"ascii" || lowered == L"english" || lowered == L"en" ||
      lowered == L"2") {
    return SrfFullscreenPolicy::Ascii;
  }
  if (lowered == L"show_ui" || lowered == L"show-ui" || lowered == L"showui" ||
      lowered == L"show" || lowered == L"overlay" || lowered == L"candidate" ||
      lowered == L"candidate_ui" || lowered == L"3") {
    return SrfFullscreenPolicy::ShowUi;
  }
  return SrfFullscreenPolicy::Off;
}

SrfCommitTransport ParseCommitTransport(const std::wstring& value) {
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered == L"auto" || lowered == L"game_auto" || lowered == L"game-auto" ||
      lowered == L"compat_auto" || lowered == L"compat-auto") {
    return SrfCommitTransport::Auto;
  }
  if (lowered == L"clipboard_paste" || lowered == L"clipboard-paste" ||
      lowered == L"clipboard" || lowered == L"paste" || lowered == L"ctrl_v" ||
      lowered == L"ctrl-v" || lowered == L"1") {
    return SrfCommitTransport::ClipboardPaste;
  }
  if (lowered == L"unicode_sendinput" || lowered == L"unicode-sendinput" ||
      lowered == L"unicode" || lowered == L"sendinput" || lowered == L"send_input" ||
      lowered == L"2") {
    return SrfCommitTransport::UnicodeSendInput;
  }
  return SrfCommitTransport::Tsf;
}

bool ParseGameCompactProfile(const std::wstring& value) {
  const std::wstring lowered = ToLower(Trim(value));
  return lowered == L"1" || lowered == L"true" || lowered == L"yes" ||
         lowered == L"on" || lowered == L"game" || lowered == L"compact" ||
         lowered == L"game_compact" || lowered == L"game-compact";
}

bool TryParseOverlayAnchor(const std::wstring& value, SrfOverlayAnchor* output) {
  if (!output) return false;
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered.empty()) return false;
  if (lowered == L"auto" || lowered == L"default") {
    *output = SrfOverlayAnchor::Auto;
  } else if (lowered == L"caret" || lowered == L"cursor") {
    *output = SrfOverlayAnchor::Caret;
  } else if (lowered == L"top_left" || lowered == L"top-left") {
    *output = SrfOverlayAnchor::TopLeft;
  } else if (lowered == L"top_center" || lowered == L"top-center") {
    *output = SrfOverlayAnchor::TopCenter;
  } else if (lowered == L"top_right" || lowered == L"top-right") {
    *output = SrfOverlayAnchor::TopRight;
  } else if (lowered == L"bottom_left" || lowered == L"bottom-left") {
    *output = SrfOverlayAnchor::BottomLeft;
  } else if (lowered == L"bottom_center" || lowered == L"bottom-center") {
    *output = SrfOverlayAnchor::BottomCenter;
  } else if (lowered == L"bottom_right" || lowered == L"bottom-right") {
    *output = SrfOverlayAnchor::BottomRight;
  } else {
    return false;
  }
  return true;
}

bool TryParseOverlayBackend(const std::wstring& value, SrfOverlayBackend* output) {
  if (!output) return false;
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered.empty()) return false;
  if (lowered == L"auto" || lowered == L"default") {
    *output = SrfOverlayBackend::Auto;
  } else if (lowered == L"in_process" || lowered == L"in-process" ||
             lowered == L"local" || lowered == L"tip") {
    *output = SrfOverlayBackend::InProcess;
  } else if (lowered == L"external" || lowered == L"out_of_process" ||
             lowered == L"out-of-process" || lowered == L"helper") {
    *output = SrfOverlayBackend::External;
  } else {
    return false;
  }
  return true;
}

bool TryNormalizeOverlayMonitor(const std::wstring& value, std::wstring* output) {
  if (!output) return false;
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered.empty()) return false;
  if (lowered == L"auto" || lowered == L"primary") {
    *output = lowered;
    return true;
  }
  int index = -1;
  if (!TryParseInt(lowered, &index) || index < 0 || index > 31) return false;
  *output = std::to_wstring(index);
  return true;
}

std::wstring ParseLearningSensitivity(const std::wstring& value, const std::wstring& fallback) {
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered == L"conservative" || lowered == L"standard" || lowered == L"aggressive") {
    return lowered;
  }
  return fallback;
}

bool TryParseFocusPolicy(const std::wstring& value, SrfFocusPolicy* out) {
  if (!out) return false;
  const std::wstring lowered = ToLower(Trim(value));
  if (lowered.empty()) return false;
  if (lowered == L"normal" || lowered == L"default" || lowered == L"global" ||
      lowered == L"off" || lowered == L"0") {
    *out = SrfFocusPolicy::Normal;
    return true;
  }
  if (lowered == L"strict" || lowered == L"stable" || lowered == L"safe" ||
      lowered == L"focus_strict") {
    *out = SrfFocusPolicy::Strict;
    return true;
  }
  if (lowered == L"window" || lowered == L"window_only" || lowered == L"window-only" ||
      lowered == L"context_window" || lowered == L"focus_window") {
    *out = SrfFocusPolicy::Window;
    return true;
  }
  return false;
}

void LoadStyle(const std::filesystem::path& path, SrfConfig& config) {
  config.style.inlinePreedit =
      ParseBool(ReadIniString(path, L"style", L"inline_preedit", config.style.inlinePreedit ? L"1" : L"0"),
                config.style.inlinePreedit);
  config.style.enhancedPosition = ParseBool(
      ReadIniString(path, L"style", L"enhanced_position", config.style.enhancedPosition ? L"1" : L"0"),
      config.style.enhancedPosition);
  config.style.pagingOnScroll = ParseBool(
      ReadIniString(path, L"style", L"paging_on_scroll", config.style.pagingOnScroll ? L"1" : L"0"),
      config.style.pagingOnScroll);
  config.style.candidateAbbreviateLength = ParseUInt(
      ReadIniString(path, L"style", L"candidate_abbreviate_length", L"64"),
      config.style.candidateAbbreviateLength);
  config.style.candidateFontSize = std::clamp(
      ParseUInt(ReadIniString(path, L"style", L"candidate_font_size", L"16"),
                config.style.candidateFontSize),
      14u, 28u);
  config.style.candidateOpacity = std::clamp(
      ParseUInt(ReadIniString(path, L"style", L"candidate_opacity", L"100"),
                config.style.candidateOpacity),
      90u, 100u);
  config.style.candidateFontFile =
      Trim(ReadIniStringLong(path, L"style", L"candidate_font_file", L"Microsoft YaHei"));
  if (config.style.candidateFontFile.empty()) {
    config.style.candidateFontFile = L"Microsoft YaHei";
  }
  config.style.candidateFontWeight = std::clamp(
      static_cast<int>(ParseUInt(ReadIniString(path, L"style", L"candidate_font_weight", L"500"),
                                 static_cast<UINT>(std::max(1, config.style.candidateFontWeight)))),
      300, 700);
  config.style.candidateSelectedFontWeight = std::clamp(
      static_cast<int>(
          ParseUInt(ReadIniString(path, L"style", L"candidate_selected_font_weight", L"600"),
                    static_cast<UINT>(std::max(1, config.style.candidateSelectedFontWeight)))),
      400, 800);
  config.style.candidateLabelFontWeight = std::clamp(
      static_cast<int>(ParseUInt(ReadIniString(path, L"style", L"candidate_label_font_weight", L"600"),
                                 static_cast<UINT>(std::max(1, config.style.candidateLabelFontWeight)))),
      400, 800);
  config.style.candidateChipFontWeight = std::clamp(
      static_cast<int>(ParseUInt(ReadIniString(path, L"style", L"candidate_chip_font_weight", L"500"),
                                 static_cast<UINT>(std::max(1, config.style.candidateChipFontWeight)))),
      350, 700);
  config.style.candidateSkinFile =
      Trim(ReadIniStringLong(path, L"style", L"candidate_skin_file", L""));
  config.style.candidateHorizontal = ParseBool(
      ReadIniString(path, L"style", L"candidate_horizontal", L"1"),
      config.style.candidateHorizontal);
  config.style.candidatePageSize = std::clamp(
      ParseUInt(ReadIniString(path, L"style", L"candidate_page_size", L"9"),
                config.style.candidatePageSize),
      3u, 9u);
  config.style.candidateHorizontalCount = std::clamp(
      ParseUInt(ReadIniString(path, L"style", L"candidate_horizontal_count", L"5"),
                config.style.candidateHorizontalCount),
      3u, 9u);
  config.style.candidateHorizontalCompact = ParseBool(
      ReadIniString(path, L"style", L"candidate_horizontal_compact",
                    config.style.candidateHorizontalCompact ? L"1" : L"0"),
      config.style.candidateHorizontalCompact);
  config.style.showCandidateReading = ParseBool(
      ReadIniString(path, L"style", L"show_candidate_reading", L"0"), config.style.showCandidateReading);
  config.style.showCandidateScore = ParseBool(
      ReadIniString(path, L"style", L"show_candidate_score", L"0"), config.style.showCandidateScore);
  config.style.highlightTypoCandidates =
      ParseBool(ReadIniString(path, SrfConfigSchema::section::kStyle,
                              SrfConfigSchema::key::kHighlightTypoCandidates, L"1"),
                config.style.highlightTypoCandidates);
  config.style.showCandidateSource =
      ParseBool(ReadIniString(path, SrfConfigSchema::section::kStyle,
                              SrfConfigSchema::key::kShowCandidateSource, L"0"),
                config.style.showCandidateSource);
  config.style.showModeInCandidateHeader = ParseBool(
      ReadIniString(path, L"style", L"show_mode_in_candidate_header", L"0"),
      config.style.showModeInCandidateHeader);
  config.style.candidateTopmost = ParseBool(
      ReadIniString(path, L"style", L"candidate_topmost",
                    config.style.candidateTopmost ? L"1" : L"0"),
      config.style.candidateTopmost);
  config.style.themeMode = ParseTheme(path);
  config.style.candidateMaterial =
      ParseCandidateMaterial(ReadIniString(path, SrfConfigSchema::section::kStyle,
                                           SrfConfigSchema::key::kCandidateMaterial,
                                           SrfConfigSchema::defaults::kCandidateMaterial));
  config.style.candidateDensity =
      ParseCandidateDensity(ReadIniString(path, SrfConfigSchema::section::kStyle,
                                          SrfConfigSchema::key::kCandidateDensity,
                                          SrfConfigSchema::defaults::kCandidateDensity));
  const std::wstring legacyLayout =
      Trim(ReadIniString(path, SrfConfigSchema::section::kStyle,
                         SrfConfigSchema::key::kCandidateLayoutVariant, L""));
  const std::wstring verticalLayout =
      Trim(ReadIniString(path, SrfConfigSchema::section::kStyle,
                         SrfConfigSchema::key::kCandidateVerticalLayoutVariant, L""));
  const std::wstring horizontalLayout =
      Trim(ReadIniString(path, SrfConfigSchema::section::kStyle,
                         SrfConfigSchema::key::kCandidateHorizontalLayoutVariant, L""));
  const bool hasExplicitLayout =
      !legacyLayout.empty() || !verticalLayout.empty() || !horizontalLayout.empty();
  const SrfCandidateLayoutVariant explicitVerticalLayout =
      ParseCandidateLayoutVariant(verticalLayout.empty()
                                      ? (legacyLayout.empty()
                                             ? SrfConfigSchema::defaults::kCandidateLayoutVariant
                                             : legacyLayout)
                                      : verticalLayout);
  const SrfCandidateLayoutVariant explicitHorizontalLayout =
      ParseCandidateLayoutVariant(horizontalLayout.empty()
                                      ? (legacyLayout.empty()
                                             ? SrfConfigSchema::defaults::kCandidateHorizontalLayoutVariant
                                             : legacyLayout)
                                      : horizontalLayout);
  config.style.candidateLayoutVariant =
      config.style.candidateHorizontal ? explicitHorizontalLayout : explicitVerticalLayout;
  const UINT configuredFontSize = config.style.candidateFontSize;
  const std::wstring configuredFontFile = config.style.candidateFontFile;
  LoadSkinFileFromConfigDir(config, path.parent_path());
  // Candidate font controls are explicit user preferences; skins must not
  // override values saved by settings.
  config.style.candidateFontSize = configuredFontSize;
  config.style.candidateFontFile = configuredFontFile;
  if (hasExplicitLayout) {
    config.style.candidateLayoutVariant =
        config.style.candidateHorizontal ? explicitHorizontalLayout : explicitVerticalLayout;
  }
}

void LoadEngine(const std::filesystem::path& path, SrfConfig& config) {
  config.engine.jianpin =
      ParseBool(ReadIniString(path, L"engine", L"jianpin", config.engine.jianpin ? L"1" : L"0"),
                config.engine.jianpin);
  config.engine.mixedPinyin = ParseBool(
      ReadIniString(path, L"engine", L"mixed_pinyin", config.engine.mixedPinyin ? L"1" : L"0"),
      config.engine.mixedPinyin);
  config.engine.mixedPinyinAggressive =
      ParseBool(ReadIniString(path, L"engine", L"mixed_pinyin_aggressive",
                              config.engine.mixedPinyinAggressive ? L"1" : L"0"),
                config.engine.mixedPinyinAggressive);
  config.engine.learningSensitivity =
      ParseLearningSensitivity(ReadIniString(path, L"engine", L"learning_sensitivity",
                                             config.engine.learningSensitivity.c_str()),
                               config.engine.learningSensitivity);
  config.engine.vAssist = ParseBool(
      ReadIniString(path, L"engine", L"v_assist", config.engine.vAssist ? L"1" : L"0"),
      config.engine.vAssist);
  config.engine.uMode =
      ParseBool(ReadIniString(path, L"engine", L"u_mode", config.engine.uMode ? L"1" : L"0"),
                config.engine.uMode);
  config.engine.retryOnFailure = ParseBool(
      ReadIniString(path, L"engine", L"retry_on_failure",
                    config.engine.retryOnFailure ? L"1" : L"0"),
      config.engine.retryOnFailure);
  config.engine.showStatusNotifications = ParseBool(
      ReadIniString(path, L"engine", L"show_status_notifications",
                    config.engine.showStatusNotifications ? L"1" : L"0"),
      config.engine.showStatusNotifications);
}

void LoadInput(const std::filesystem::path& path, SrfConfig& config) {
  config.input.defaultAscii =
      ParseBool(ReadIniString(path, L"input", L"default_ascii", L"0"), config.input.defaultAscii);
  config.input.defaultFullShape = ParseBool(
      ReadIniString(path, L"input", L"default_full_shape", L"0"), config.input.defaultFullShape);
  config.input.defaultChinesePunct = ParseBool(
      ReadIniString(path, L"input", L"default_chinese_punct", L"1"), config.input.defaultChinesePunct);
  config.input.defaultFuzzyPinyin = ParseBool(
      ReadIniString(path, L"input", L"default_fuzzy_pinyin", L"0"), config.input.defaultFuzzyPinyin);
  config.input.defaultDoublePinyin = ParseBool(
      ReadIniString(path, L"input", L"default_double_pinyin", L"0"), config.input.defaultDoublePinyin);
  config.input.curlyPunct =
      ParseBool(ReadIniString(path, L"input", L"curly_punct", L"1"), config.input.curlyPunct);
  config.input.autoPairPunct =
      ParseBool(ReadIniString(path, L"input", L"auto_pair_punct", L"1"), config.input.autoPairPunct);
  config.input.numberFullwidth = ParseBool(
      ReadIniString(path, L"input", L"number_fullwidth", L"0"), config.input.numberFullwidth);
  config.input.symbolFullwidth = ParseBool(
      ReadIniString(path, L"input", L"symbol_fullwidth", L"1"), config.input.symbolFullwidth);
  config.input.shiftSymbolTemporaryAscii =
      ParseBool(ReadIniString(path, L"input", L"shift_symbol_temporary_ascii", L"0"),
                config.input.shiftSymbolTemporaryAscii);
  config.input.dateAutoFormat = ParseBool(
      ReadIniString(path, L"input", L"date_auto_format", L"1"), config.input.dateAutoFormat);
  config.input.englishWordInput = ParseBool(
      ReadIniString(path, L"input", L"english_word_input", L"0"), config.input.englishWordInput);
  config.input.symbolToolbox =
      ParseBool(ReadIniString(path, L"input", L"symbol_toolbox", L"1"), config.input.symbolToolbox);
  config.input.emojiInput =
      ParseBool(ReadIniString(path, L"input", L"emoji_input", L"1"), config.input.emojiInput);
  config.input.traditionalOutput =
      ParseBool(ReadIniString(path, L"input", L"traditional_output", L"0"),
                config.input.traditionalOutput);
  config.input.cnEnHotkey =
      ParseCnEnHotkey(ReadIniString(path, L"input", L"cn_en_hotkey", L"none"),
                      config.input.cnEnHotkey);
  config.input.fullShapeHotkeyEnabled = ParseBool(
      ReadIniString(path, L"input", L"full_shape_hotkey", L"0"),
      config.input.fullShapeHotkeyEnabled);
  config.input.punctHotkeyEnabled =
      ParseBool(ReadIniString(path, L"input", L"punct_hotkey", L"0"),
                config.input.punctHotkeyEnabled);
  config.input.fuzzyHotkeyEnabled =
      ParseBool(ReadIniString(path, L"input", L"fuzzy_hotkey", L"0"),
                config.input.fuzzyHotkeyEnabled);
  config.input.doubleHotkeyEnabled =
      ParseBool(ReadIniString(path, L"input", L"double_pinyin_hotkey", L"0"),
                config.input.doubleHotkeyEnabled);
  config.input.shiftTapHotkeyEnabled =
      ParseBool(ReadIniString(path, L"input", L"shift_tap_hotkey", L"1"),
                config.input.shiftTapHotkeyEnabled);
  config.input.candidateNumberSelect =
      ParseBool(ReadIniString(path, L"input", L"candidate_number_select", L"1"),
                config.input.candidateNumberSelect);
  config.input.candidateLeftClick =
      ParseBool(ReadIniString(path, L"input", L"candidate_left_click", L"1"),
                config.input.candidateLeftClick);
  config.input.candidateRightClick =
      ParseBool(ReadIniString(path, L"input", L"candidate_right_click", L"1"),
                config.input.candidateRightClick);
  config.style.candidateLeftClick = config.input.candidateLeftClick;
  config.style.candidateRightClick = config.input.candidateRightClick;
  config.input.pageMinusEqual =
      ParseBool(ReadIniString(path, L"input", L"page_minus_equal", L"1"), config.input.pageMinusEqual);
  config.input.pageCommaPeriod = ParseBool(
      ReadIniString(path, L"input", L"page_comma_period", L"1"), config.input.pageCommaPeriod);
  config.input.pagePgUpDown =
      ParseBool(ReadIniString(path, L"input", L"page_pgup_pgdn", L"1"), config.input.pagePgUpDown);

  SrfHotkeyOptions traditionalHotkey = config.input.traditionalHotkey;
  if (TryParseHotkey(ReadIniString(path, L"input", L"traditional_hotkey", L"off"),
                     'F', TF_MOD_CONTROL | TF_MOD_SHIFT | TF_MOD_ALT, &traditionalHotkey)) {
    config.input.traditionalHotkey = traditionalHotkey;
  }
  SrfHotkeyOptions gameModeHotkey = config.input.gameModeHotkey;
  if (TryParseHotkey(ReadIniString(path, L"input", L"game_mode_hotkey", L"off"),
                     'G', TF_MOD_CONTROL | TF_MOD_SHIFT | TF_MOD_ALT, &gameModeHotkey)) {
    config.input.gameModeHotkey = gameModeHotkey;
  }
  SrfHotkeyOptions temporaryAsciiHotkey = config.input.temporaryAsciiHotkey;
  if (TryParseHotkey(
          ReadIniString(path, L"input", L"temporary_ascii_hotkey", L"off"),
          VK_SPACE, TF_MOD_CONTROL | TF_MOD_SHIFT, &temporaryAsciiHotkey)) {
    config.input.temporaryAsciiHotkey = temporaryAsciiHotkey;
  }
}

void LoadClipboard(const std::filesystem::path& path, SrfConfig& config) {
  config.clipboard.backgroundEnabled = ParseBool(
      ReadIniString(path, L"clipboard", L"background_enabled", L"0"), config.clipboard.backgroundEnabled);
  config.clipboard.maxHistoryItems = std::clamp(
      ParseUInt(ReadIniString(path, L"clipboard", L"max_history_items", L"60"),
                config.clipboard.maxHistoryItems),
      0u, 300u);
  config.clipboard.maxPinnedItems = std::clamp(
      ParseUInt(ReadIniString(path, L"clipboard", L"max_pinned_items", L"24"),
                config.clipboard.maxPinnedItems),
      0u, 100u);
  config.clipboard.maxTextUtf16Units = std::clamp(
      ParseUInt(ReadIniString(path, L"clipboard", L"max_text_utf16_units", L"20000"),
                config.clipboard.maxTextUtf16Units),
      20u, 20000u);
}

void LoadPrivacy(const std::filesystem::path& path, SrfConfig& config) {
  config.privacy.enabled = ParseBool(
      ReadIniString(path, SrfConfigSchema::section::kPrivacy,
                    SrfConfigSchema::key::kPrivacyEnabled, L"0"),
      config.privacy.enabled);
  config.privacy.neverLearnProcessList =
      ReadIniList(path, SrfConfigSchema::section::kPrivacy,
                  SrfConfigSchema::key::kNeverLearnProcesses);
  config.privacy.neverClipboardProcessList =
      ReadIniList(path, SrfConfigSchema::section::kPrivacy,
                  SrfConfigSchema::key::kNeverClipboardProcesses);
  config.privacy.neverCandidateProcessList =
      ReadIniList(path, SrfConfigSchema::section::kPrivacy,
                  SrfConfigSchema::key::kNeverCandidateProcesses);
}

void LoadScreenshot(const std::filesystem::path& path, SrfConfig& config) {
  config.screenshot.hotkey.enabled = false;
  config.screenshot.hotkey.vk = 'A';
  config.screenshot.hotkey.modifiers = TF_MOD_CONTROL | TF_MOD_SHIFT | TF_MOD_ALT;

  SrfHotkeyOptions parsed = config.screenshot.hotkey;
  if (TryParseHotkey(ReadIniString(path, L"screenshot", L"hotkey", L"off"), 'A',
                     TF_MOD_CONTROL | TF_MOD_SHIFT | TF_MOD_ALT, &parsed)) {
    config.screenshot.hotkey = parsed;
  }

  config.screenshot.saveDir = Trim(ReadIniStringLong(path, L"screenshot", L"save_dir", L""));
  std::wstring format = ToLower(Trim(ReadIniString(path, L"screenshot", L"format", L"png")));
  if (format != L"jpg" && format != L"jpeg" && format != L"bmp") format = L"png";
  if (format == L"jpeg") format = L"jpg";
  config.screenshot.format = std::move(format);
}

void LoadCompatibility(const std::filesystem::path& path, SrfConfig& config) {
  config.compatibility.fullscreenDetection =
      ParseBool(ReadIniString(path, SrfConfigSchema::section::kCompatibility,
                              SrfConfigSchema::key::kFullscreenDetection, L"1"),
                config.compatibility.fullscreenDetection);
  config.compatibility.fullscreenPolicy = ParseFullscreenPolicy(
      ReadIniString(path, SrfConfigSchema::section::kCompatibility,
                    SrfConfigSchema::key::kFullscreenPolicy,
                    SrfConfigSchema::defaults::kFullscreenPolicy));
  config.compatibility.commitTransport = ParseCommitTransport(
      ReadIniString(path, SrfConfigSchema::section::kCompatibility,
                    SrfConfigSchema::key::kCommitTransport,
                    SrfConfigSchema::defaults::kCommitTransport));
  config.compatibility.builtinGameList =
      ParseBool(ReadIniString(path, SrfConfigSchema::section::kCompatibility,
                              SrfConfigSchema::key::kBuiltinGameList, L"1"),
                config.compatibility.builtinGameList);
  config.compatibility.autoSuggestAppOptions =
      ParseBool(ReadIniString(path, SrfConfigSchema::section::kCompatibility,
                              SrfConfigSchema::key::kAutoSuggestAppOptions, L"1"),
                config.compatibility.autoSuggestAppOptions);
  config.compatibility.gameProcessList =
      ReadIniList(path, SrfConfigSchema::section::kCompatibility,
                  SrfConfigSchema::key::kGameProcesses);
  if (config.compatibility.builtinGameList) {
    AppendDefaultGameProcessPatterns(config.compatibility.gameProcessList);
  }
}

void LoadGeneral(const std::filesystem::path& path, SrfConfig& config) {
  config.globalAscii =
      ParseBool(ReadIniString(path, L"general", L"global_ascii", config.globalAscii ? L"1" : L"0"),
                config.globalAscii);

  const std::wstring showValue = ReadIniString(path, L"general", L"show_notifications", L"true");
  const std::wstring lowered = ToLower(Trim(showValue));
  if (lowered == L"true" || lowered == L"1" || lowered == L"yes" || lowered == L"on") {
    config.showNotifications = true;
    config.notificationKinds.clear();
  } else if (lowered == L"false" || lowered == L"0" || lowered == L"no" || lowered == L"off") {
    config.showNotifications = false;
    config.notificationKinds.clear();
  } else {
    config.showNotifications = true;
    config.notificationKinds = ReadIniList(path, L"general", L"show_notifications");
  }

  config.showNotificationsTimeMs =
      ParseUInt(ReadIniString(path, L"general", L"show_notifications_time", L"1200"),
                config.showNotificationsTimeMs);
}

void LoadAppOptions(const std::filesystem::path& path, SrfConfig& config) {
  const std::vector<std::wstring> sections = ReadIniSectionNames(path);
  for (const auto& section : sections) {
    const std::wstring lowered = ToLower(section);
    if (lowered.rfind(L"app:", 0) == 0 && section.size() > 4) {
      const std::wstring appName = section.substr(4);
      if (!ParseBool(ReadIniString(path, section.c_str(), L"enabled", L"1"), true)) {
        continue;
      }
      SrfAppOptions options = {};

      const std::wstring appPolicy =
          ToLower(Trim(ReadIniString(path, section.c_str(), L"policy")));
      if (appPolicy == L"show_ui" || appPolicy == L"show-ui" ||
          appPolicy == L"showui" || appPolicy == L"show" ||
          appPolicy == L"overlay" || appPolicy == L"candidate" ||
          appPolicy == L"candidate_ui") {
        options.hasAsciiMode = true;
        options.asciiMode = false;
        options.hasHideUi = true;
        options.hideUi = false;
        options.hasCandidateTopmost = true;
        options.candidateTopmost = true;
      } else if (appPolicy == L"ascii" || appPolicy == L"ascii_mode" ||
                 appPolicy == L"english" || appPolicy == L"en") {
        options.hasAsciiMode = true;
        options.asciiMode = true;
      } else if (appPolicy == L"hide" || appPolicy == L"hide_ui" ||
                 appPolicy == L"hide-ui" || appPolicy == L"hideui") {
        options.hasHideUi = true;
        options.hideUi = true;
      } else if (appPolicy == L"topmost_off" ||
                 appPolicy == L"candidate_topmost_off") {
        options.hasCandidateTopmost = true;
        options.candidateTopmost = false;
      }

      const std::wstring asciiMode = ReadIniString(path, section.c_str(), L"ascii_mode");
      if (!Trim(asciiMode).empty()) {
        options.hasAsciiMode = true;
        options.asciiMode = ParseBool(asciiMode, false);
      }

      const std::wstring hideUi = ReadIniString(path, section.c_str(), L"hide_ui");
      if (!Trim(hideUi).empty()) {
        options.hasHideUi = true;
        options.hideUi = ParseBool(hideUi, false);
      }

      const std::wstring inlinePreedit = ReadIniString(path, section.c_str(), L"inline_preedit");
      if (!Trim(inlinePreedit).empty()) {
        options.hasInlinePreedit = true;
        options.inlinePreedit = ParseBool(inlinePreedit, true);
      }

      const std::wstring enhancedPosition =
          ReadIniString(path, section.c_str(), L"enhanced_position");
      if (!Trim(enhancedPosition).empty()) {
        options.hasEnhancedPosition = true;
        options.enhancedPosition = ParseBool(enhancedPosition, true);
      }

      const std::wstring candidateTopmost =
          ReadIniString(path, section.c_str(), L"candidate_topmost");
      if (!Trim(candidateTopmost).empty()) {
        options.hasCandidateTopmost = true;
        options.candidateTopmost = ParseBool(candidateTopmost, true);
      }

      const std::wstring commitTransport =
          ReadIniString(path, section.c_str(), L"commit_transport");
      if (!Trim(commitTransport).empty()) {
        options.hasCommitTransport = true;
        options.commitTransport = ParseCommitTransport(commitTransport);
      }

      const std::wstring gameProfile =
          ReadIniString(path, section.c_str(), SrfConfigSchema::key::kGameProfile);
      const std::wstring candidateProfile =
          ReadIniString(path, section.c_str(), L"candidate_profile");
      const std::wstring genericProfile = ReadIniString(path, section.c_str(), L"profile");
      const std::wstring profile =
          !gameProfile.empty() ? gameProfile
                               : (!candidateProfile.empty() ? candidateProfile : genericProfile);
      if (!Trim(profile).empty()) {
        options.hasGameProfile = true;
        options.gameCompactProfile = ParseGameCompactProfile(profile);
      }

      SrfOverlayAnchor overlayAnchor = SrfOverlayAnchor::Auto;
      const std::wstring overlayAnchorValue = ReadIniString(
          path, section.c_str(), SrfConfigSchema::key::kOverlayAnchor);
      if (TryParseOverlayAnchor(overlayAnchorValue, &overlayAnchor)) {
        options.hasOverlayAnchor = true;
        options.overlayAnchor = overlayAnchor;
      }

      int overlayOffset = 0;
      const std::wstring overlayOffsetX = ReadIniString(
          path, section.c_str(), SrfConfigSchema::key::kOverlayOffsetX);
      if (TryParseInt(overlayOffsetX, &overlayOffset)) {
        options.hasOverlayOffsetX = true;
        options.overlayOffsetX = std::clamp(overlayOffset, -4000, 4000);
      }
      const std::wstring overlayOffsetY = ReadIniString(
          path, section.c_str(), SrfConfigSchema::key::kOverlayOffsetY);
      if (TryParseInt(overlayOffsetY, &overlayOffset)) {
        options.hasOverlayOffsetY = true;
        options.overlayOffsetY = std::clamp(overlayOffset, -4000, 4000);
      }

      const std::wstring overlayScale = ReadIniString(
          path, section.c_str(), SrfConfigSchema::key::kOverlayScale);
      if (!Trim(overlayScale).empty()) {
        options.hasOverlayScale = true;
        options.overlayScalePercent = std::clamp(ParseUInt(overlayScale, 100), 50u, 200u);
      }

      std::wstring overlayMonitor;
      if (TryNormalizeOverlayMonitor(
              ReadIniString(path, section.c_str(), SrfConfigSchema::key::kOverlayMonitor),
              &overlayMonitor)) {
        options.hasOverlayMonitor = true;
        options.overlayMonitor = std::move(overlayMonitor);
      }

      SrfOverlayBackend overlayBackend = SrfOverlayBackend::Auto;
      if (TryParseOverlayBackend(
              ReadIniString(path, section.c_str(), SrfConfigSchema::key::kOverlayBackend),
              &overlayBackend)) {
        options.hasOverlayBackend = true;
        options.overlayBackend = overlayBackend;
      }

      SrfFocusPolicy focusPolicy = SrfFocusPolicy::Normal;
      const std::wstring explicitFocusPolicy =
          ReadIniString(path, section.c_str(), L"focus_policy");
      if (TryParseFocusPolicy(explicitFocusPolicy, &focusPolicy)) {
        options.hasFocusPolicy = true;
        options.focusPolicy = focusPolicy;
      } else {
        if (TryParseFocusPolicy(appPolicy, &focusPolicy)) {
          options.hasFocusPolicy = true;
          options.focusPolicy = focusPolicy;
        }
      }

      config.appOptions[appName] = options;
    }
  }
}

SrfConfig LoadSrfConfigFromPathInternal(const std::filesystem::path& path) {
  SrfConfig config = {};
  if (path.empty() || !std::filesystem::exists(path)) return config;

  const IniDocument ini = LoadIniDocument(path);
  ActiveIniDocumentScope activeIni(ini);
  LoadGeneral(path, config);
  LoadStyle(path, config);
  LoadEngine(path, config);
  LoadInput(path, config);
  LoadClipboard(path, config);
  LoadPrivacy(path, config);
  LoadScreenshot(path, config);
  LoadCompatibility(path, config);
  LoadAppOptions(path, config);
  return config;
}

void RefreshConfigCache() {
  const std::filesystem::path path = ResolveSrfConfigPathInternal();
  std::error_code ec;
  const bool exists = !path.empty() && std::filesystem::exists(path, ec);
  bool hasWriteTime = false;
  std::filesystem::file_time_type writeTime = {};
  if (exists && !ec) {
    writeTime = std::filesystem::last_write_time(path, ec);
    hasWriteTime = !ec;
  }

  {
    ConfigCache& cache = GetConfigCache();
    std::lock_guard<std::mutex> lock(cache.mutex);
    if (cache.initialized && path == cache.path && hasWriteTime == cache.hasWriteTime &&
        (!hasWriteTime || writeTime == cache.writeTime)) {
      return;
    }
  }

  SrfConfig config = LoadSrfConfigFromPathInternal(path);

  ConfigCache& cache = GetConfigCache();
  std::lock_guard<std::mutex> lock(cache.mutex);
  const bool changed = !cache.initialized || path != cache.path ||
                       hasWriteTime != cache.hasWriteTime ||
                       (!hasWriteTime || writeTime != cache.writeTime);
  cache.config = std::move(config);
  cache.path = path;
  cache.writeTime = writeTime;
  cache.hasWriteTime = hasWriteTime;
  cache.initialized = true;
  if (changed) {
    ++cache.version;
    GetConfigVersionAtomic().store(cache.version, std::memory_order_release);
  }
}

std::filesystem::path CurrentConfigWatchDirectory() {
  const std::filesystem::path path = ResolveSrfConfigPathInternal();
  if (path.empty()) return {};
  std::error_code ec;
  std::filesystem::path dir = path.parent_path();
  if (dir.empty() || !std::filesystem::exists(dir, ec) || ec) return {};
  return dir;
}

bool WaitForConfigDirectoryChange(const std::filesystem::path& dir) {
  if (dir.empty()) return false;
  HANDLE directory = CreateFileW(dir.c_str(), FILE_LIST_DIRECTORY,
                                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
                                OPEN_EXISTING,
                                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED, nullptr);
  if (directory == INVALID_HANDLE_VALUE) return false;

  HANDLE event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (!event) {
    CloseHandle(directory);
    return false;
  }

  BYTE buffer[4096] = {};
  OVERLAPPED overlapped = {};
  overlapped.hEvent = event;
  const BOOL started = ReadDirectoryChangesW(
      directory, buffer, sizeof(buffer), FALSE,
      FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_SIZE |
          FILE_NOTIFY_CHANGE_CREATION,
      nullptr, &overlapped, nullptr);
  if (!started) {
    CloseHandle(event);
    CloseHandle(directory);
    return false;
  }

  const DWORD wait = WaitForSingleObject(event, kConfigWatchFallbackIntervalMs);
  DWORD transferred = 0;
  if (wait == WAIT_TIMEOUT) {
    CancelIoEx(directory, &overlapped);
    WaitForSingleObject(event, 200);
    GetOverlappedResult(directory, &overlapped, &transferred, FALSE);
    CloseHandle(event);
    CloseHandle(directory);
    return true;
  }

  const bool changed =
      wait == WAIT_OBJECT_0 && GetOverlappedResult(directory, &overlapped, &transferred, FALSE);
  CloseHandle(event);
  CloseHandle(directory);
  if (changed) Sleep(kConfigWatchDebounceMs);
  return changed;
}

void ConfigWatcherLoop() {
  while (true) {
    RefreshConfigCache();
    if (!WaitForConfigDirectoryChange(CurrentConfigWatchDirectory())) {
      Sleep(kConfigWatchFallbackIntervalMs);
    }
  }
}

void EnsureConfigWatcherStarted() {
  static std::once_flag once;
  std::call_once(once, []() {
    RefreshConfigCache();
    SrfTip_BackgroundWorkerAddRef();
    try {
      std::thread(ConfigWatcherLoop).detach();
    } catch (...) {
      SrfTip_BackgroundWorkerRelease();
    }
  });
}

}  // namespace

SrfConfig LoadSrfConfigFromPath(const std::filesystem::path& path) {
  return LoadSrfConfigFromPathInternal(path);
}

// Skin file parsing lives outside the anonymous namespace so it can call
// the public LoadSkinFile declaration from the header.

int HexDigit(char ch) {
  if (ch >= '0' && ch <= '9') return ch - '0';
  if (ch >= 'a' && ch <= 'f') return ch - 'a' + 10;
  if (ch >= 'A' && ch <= 'F') return ch - 'A' + 10;
  return -1;
}

bool ParseHexByte(const std::string& text, size_t offset, BYTE& out) {
  if (offset + 1 >= text.size()) return false;
  const int hi = HexDigit(text[offset]);
  const int lo = HexDigit(text[offset + 1]);
  if (hi < 0 || lo < 0) return false;
  out = static_cast<BYTE>((hi << 4) | lo);
  return true;
}

COLORREF ParseColorHex(const std::string& hex) {
  std::string h = hex;
  if (!h.empty() && h[0] == '#') h = h.substr(1);
  if (h.size() != 6 && h.size() != 8) return CLR_INVALID;
  BYTE r = 0, g = 0, b = 0;
  if (!ParseHexByte(h, 0, r) || !ParseHexByte(h, 2, g) || !ParseHexByte(h, 4, b)) {
    return CLR_INVALID;
  }
  return RGB(r, g, b);
}

float ParseFloatStr(const std::string& s, float fallback) {
  try {
    return std::stof(s);
  } catch (...) {
    return fallback;
  }
}

int ParseIntStr(const std::string& s, int fallback) {
  try {
    return std::stoi(s);
  } catch (...) {
    return fallback;
  }
}

bool ParseBoolStr(const std::string& s, bool fallback) {
  std::wstring ws(s.begin(), s.end());
  return ParseBool(ws, fallback);
}

size_t SkipJsonWhitespace(const std::string& json, size_t pos) {
  while (pos < json.size() &&
         (json[pos] == ' ' || json[pos] == '\t' || json[pos] == '\n' || json[pos] == '\r')) {
    ++pos;
  }
  return pos;
}

bool ReadJsonHex4(const std::string& json, size_t pos, uint32_t& out) {
  if (pos + 4 > json.size()) return false;
  uint32_t value = 0;
  for (size_t i = 0; i < 4; ++i) {
    const int digit = HexDigit(json[pos + i]);
    if (digit < 0) return false;
    value = (value << 4) | static_cast<uint32_t>(digit);
  }
  out = value;
  return true;
}

void AppendUtf8(std::string& out, uint32_t codepoint) {
  if (codepoint <= 0x7F) {
    out.push_back(static_cast<char>(codepoint));
  } else if (codepoint <= 0x7FF) {
    out.push_back(static_cast<char>(0xC0 | (codepoint >> 6)));
    out.push_back(static_cast<char>(0x80 | (codepoint & 0x3F)));
  } else if (codepoint <= 0xFFFF) {
    out.push_back(static_cast<char>(0xE0 | (codepoint >> 12)));
    out.push_back(static_cast<char>(0x80 | ((codepoint >> 6) & 0x3F)));
    out.push_back(static_cast<char>(0x80 | (codepoint & 0x3F)));
  } else if (codepoint <= 0x10FFFF) {
    out.push_back(static_cast<char>(0xF0 | (codepoint >> 18)));
    out.push_back(static_cast<char>(0x80 | ((codepoint >> 12) & 0x3F)));
    out.push_back(static_cast<char>(0x80 | ((codepoint >> 6) & 0x3F)));
    out.push_back(static_cast<char>(0x80 | (codepoint & 0x3F)));
  }
}

bool ParseJsonStringAt(const std::string& json, size_t start, std::string& out, size_t& next) {
  if (start >= json.size() || json[start] != '"') return false;
  out.clear();
  size_t pos = start + 1;
  while (pos < json.size()) {
    const char ch = json[pos++];
    if (ch == '"') {
      next = pos;
      return true;
    }
    if (ch != '\\') {
      out.push_back(ch);
      continue;
    }
    if (pos >= json.size()) return false;
    const char esc = json[pos++];
    switch (esc) {
      case '"': out.push_back('"'); break;
      case '\\': out.push_back('\\'); break;
      case '/': out.push_back('/'); break;
      case 'b': out.push_back('\b'); break;
      case 'f': out.push_back('\f'); break;
      case 'n': out.push_back('\n'); break;
      case 'r': out.push_back('\r'); break;
      case 't': out.push_back('\t'); break;
      case 'u': {
        uint32_t cp = 0;
        if (!ReadJsonHex4(json, pos, cp)) return false;
        pos += 4;
        if (cp >= 0xD800 && cp <= 0xDBFF) {
          if (pos + 6 > json.size() || json[pos] != '\\' || json[pos + 1] != 'u') return false;
          uint32_t low = 0;
          if (!ReadJsonHex4(json, pos + 2, low) || low < 0xDC00 || low > 0xDFFF) return false;
          pos += 6;
          cp = 0x10000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
        }
        AppendUtf8(out, cp);
        break;
      }
      default:
        return false;
    }
  }
  return false;
}

std::string ParseJsonBareValue(const std::string& json, size_t start, size_t& next) {
  size_t end = start;
  while (end < json.size() && json[end] != ',' && json[end] != '}' && json[end] != ']') {
    ++end;
  }
  next = end;
  std::string value = json.substr(start, end - start);
  while (!value.empty() && (value.back() == ' ' || value.back() == '\t' ||
                            value.back() == '\n' || value.back() == '\r')) {
    value.pop_back();
  }
  return value;
}

std::string SimpleJsonGetValue(const std::string& json, const std::string& key) {
  size_t pos = 0;
  while (pos < json.size()) {
    if (json[pos] != '"') {
      ++pos;
      continue;
    }

    std::string parsedKey;
    size_t afterKey = 0;
    if (!ParseJsonStringAt(json, pos, parsedKey, afterKey)) {
      ++pos;
      continue;
    }

    size_t valueStart = SkipJsonWhitespace(json, afterKey);
    if (valueStart >= json.size() || json[valueStart] != ':') {
      pos = afterKey;
      continue;
    }
    valueStart = SkipJsonWhitespace(json, valueStart + 1);
    if (valueStart >= json.size()) return {};

    if (parsedKey == key) {
      if (json[valueStart] == '"') {
        std::string value;
        size_t afterValue = 0;
        return ParseJsonStringAt(json, valueStart, value, afterValue) ? value : std::string{};
      }
      size_t afterValue = 0;
      return ParseJsonBareValue(json, valueStart, afterValue);
    }

    pos = valueStart;
  }
  return {};
}

// YAML simple parser: "key: value" lines
std::string SimpleYamlGetValue(const std::string& yaml, const std::string& key) {
  std::string needle = key + ":";
  size_t pos = 0;
  while (pos < yaml.size()) {
    size_t found = yaml.find(needle, pos);
    if (found == std::string::npos) break;
    // Make sure it's at start of line or after whitespace
    if (found > 0 && yaml[found - 1] != '\n') {
      pos = found + 1;
      continue;
    }
    size_t valStart = found + needle.size();
    while (valStart < yaml.size() && (yaml[valStart] == ' ' || yaml[valStart] == '\t'))
      ++valStart;
    size_t valEnd = valStart;
    while (valEnd < yaml.size() && yaml[valEnd] != '\n' && yaml[valEnd] != '\r')
      ++valEnd;
    std::string val = yaml.substr(valStart, valEnd - valStart);
    // Strip quotes
    if (val.size() >= 2 && val.front() == '"' && val.back() == '"')
      val = val.substr(1, val.size() - 2);
    return val;
  }
  return {};
}

void ApplySkinColor(const std::string& content, const std::string& key, bool isYaml,
                    COLORREF& outField) {
  std::string val = isYaml ? SimpleYamlGetValue(content, key) : SimpleJsonGetValue(content, key);
  if (!val.empty()) {
    COLORREF c = ParseColorHex(val);
    if (c != CLR_INVALID) outField = c;
  }
}

void ApplySkinFloat(const std::string& content, const std::string& key, bool isYaml,
                    float& outField) {
  std::string val = isYaml ? SimpleYamlGetValue(content, key) : SimpleJsonGetValue(content, key);
  if (!val.empty()) outField = ParseFloatStr(val, outField);
}

void ApplySkinInt(const std::string& content, const std::string& key, bool isYaml,
                  int& outField) {
  std::string val = isYaml ? SimpleYamlGetValue(content, key) : SimpleJsonGetValue(content, key);
  if (!val.empty()) outField = ParseIntStr(val, outField);
}

void ApplySkinBool(const std::string& content, const std::string& key, bool isYaml,
                   bool& outField) {
  std::string val = isYaml ? SimpleYamlGetValue(content, key) : SimpleJsonGetValue(content, key);
  if (!val.empty()) outField = ParseBoolStr(val, outField);
}

void LoadSkinFileFromConfigDir(SrfConfig& config, const std::filesystem::path& configDir) {
  // Reset skin fields
  SrfUIStyle& st = config.style;
  st.skinLoaded = false;
  st.skinWindowBg = CLR_INVALID;
  st.skinWindowBgTo = CLR_INVALID;
  st.skinHeaderBg = CLR_INVALID;
  st.skinHeaderBgTo = CLR_INVALID;
  st.skinBorder = CLR_INVALID;
  st.skinDivider = CLR_INVALID;
  st.skinText = CLR_INVALID;
  st.skinMutedText = CLR_INVALID;
  st.skinBadgeBg = CLR_INVALID;
  st.skinBadgeBorder = CLR_INVALID;
  st.skinBadgeText = CLR_INVALID;
  st.skinHoverBg = CLR_INVALID;
  st.skinHoverBorder = CLR_INVALID;
  st.skinItemBg = CLR_INVALID;
  st.skinItemBorder = CLR_INVALID;
  st.skinSelectedBg = CLR_INVALID;
  st.skinSelectedBorder = CLR_INVALID;
  st.skinPressedBg = CLR_INVALID;
  st.skinPressedBorder = CLR_INVALID;
  st.skinSelectedText = CLR_INVALID;
  st.skinSelectedMutedText = CLR_INVALID;
  st.skinChipBg = CLR_INVALID;
  st.skinChipBorder = CLR_INVALID;
  st.skinChipText = CLR_INVALID;
  st.skinChipActiveBg = CLR_INVALID;
  st.skinChipActiveBorder = CLR_INVALID;
  st.skinChipActiveText = CLR_INVALID;
  st.skinSelectedOutline = CLR_INVALID;
  st.skinSelectedAccentWidth = -1;
  st.skinSelectedRingOpacity = -1.0f;
  st.skinSelectedIndicator.clear();
  st.skinBorderOpacity = 1.0f;
  st.skinDividerOpacity = 1.0f;
  st.skinShadowOpacity = 0.0f;
  st.skinShadowSize = 0;
  st.skinShadowEnabled = true;
  st.skinFontWeight = -1;
  st.skinSelectedFontWeight = -1;
  st.skinLabelFontWeight = -1;
  st.skinChipFontWeight = -1;
  st.skinCornerRadius = -1;
  st.skinHeaderCornerRadius = -1;
  st.skinRowCornerRadius = -1;
  st.skinBadgeCornerRadius = -1;
  st.skinOuterPadX = -1;
  st.skinOuterPadY = -1;
  st.skinHeaderPadX = -1;
  st.skinHeaderPadY = -1;
  st.skinHeaderGap = -1;
  st.skinItemGap = -1;
  st.skinItemPadX = -1;
  st.skinItemPadY = -1;
  st.skinLabelWidth = -1;
  st.skinLabelGap = -1;
  st.skinCommentGap = -1;
  st.skinMinWidth = -1;
  st.skinPreferredWidth = -1;
  st.skinMaxWidth = -1;
  st.skinMinHorizontalCardWidth = -1;
  st.skinMaxHorizontalCardWidth = -1;

  if (st.candidateSkinFile.empty()) return;

  auto resolveThemeFileInsideDir = [&](const std::filesystem::path& dir) -> std::filesystem::path {
    // Prefer theme.json, then theme.yaml/yml.
    for (const auto& name : {L"theme.json", L"theme.yaml", L"theme.yml"}) {
      std::filesystem::path candidate = dir / name;
      std::error_code ec;
      if (std::filesystem::exists(candidate, ec) && !ec) return candidate;
    }
    return {};
  };

  // Resolve skin file path from config/runtime roots.
  // `candidate_skin_file` can be:
  // - a skin directory: skins/dark/ (contains theme.json)
  // - a skin name: dark  (resolved to skins/dark/theme.json)
  std::filesystem::path skinPath(Trim(st.candidateSkinFile));

  // 先做安全校验：若用户配置成 UNC/不可达网络路径，直接降级为默认皮肤，
  // 避免 std::filesystem::exists / ifstream 在 IME 线程上阻塞导致系统卡死。
  // 相对路径先不判定（后续会解析到本地 roots），绝对路径必须是本地盘符。
  if (skinPath.is_absolute() && !IsAllowedLocalSkinPath(skinPath)) {
    return;
  }

  if (!skinPath.is_absolute()) {
    std::vector<std::filesystem::path> roots;
    if (!configDir.empty()) roots.push_back(configDir);

    // Important: use our module directory (TIP DLL), not host process EXE (e.g. ctfmon.exe).
    const std::filesystem::path moduleDir = ModuleDir();
    if (!moduleDir.empty()) {
      std::filesystem::path current = moduleDir;
      for (int depth = 0; depth < 6 && !current.empty(); ++depth) {
        roots.push_back(current);
        const std::filesystem::path parent = current.parent_path();
        if (parent.empty() || parent == current) break;
        current = parent;
      }
    }

    for (const auto& root : roots) {
      for (const auto& dir : {root, root / L"skins"}) {
        auto candidate = dir / skinPath;
        std::error_code ec;
        // Accept directory or name (directory without suffix).
        if (std::filesystem::exists(candidate, ec) && !ec) {
          skinPath = candidate;
          break;
        }
        // If user configured a bare name like "dark", try skins/dark/theme.*
        if (candidate.extension().empty()) {
          const auto theme = resolveThemeFileInsideDir(candidate);
          if (!theme.empty()) {
            skinPath = theme;
            break;
          }
        }
      }
      if (skinPath.is_absolute()) break;
    }
  }

  std::error_code ec;
  if (skinPath.empty()) return;
  // 再次校验：相对路径解析出来后，仍必须是本地路径（拒绝任何 UNC/网络形式）。
  if (skinPath.is_absolute() && !IsAllowedLocalSkinPath(skinPath)) {
    return;
  }
  if (std::filesystem::exists(skinPath, ec) && !ec) {
    if (std::filesystem::is_directory(skinPath, ec) && !ec) {
      skinPath = resolveThemeFileInsideDir(skinPath);
    }
  } else {
    // If nothing exists at the resolved path and it looks like a name, try skins/<name>/theme.*
    if (skinPath.extension().empty()) {
      const auto theme = resolveThemeFileInsideDir(skinPath);
      if (!theme.empty()) skinPath = theme;
    }
  }

  ec.clear();
  if (skinPath.empty() || !std::filesystem::exists(skinPath, ec) || ec) return;

  // Read file content
  std::ifstream ifs(skinPath, std::ios::binary);
  if (!ifs) return;
  std::string content((std::istreambuf_iterator<char>(ifs)), std::istreambuf_iterator<char>());
  ifs.close();

  const bool isYaml = skinPath.extension() == L".yaml" || skinPath.extension() == L".yml";

  // Material and layout overrides from skin
  {
    std::string mat = isYaml ? SimpleYamlGetValue(content, "material")
                              : SimpleJsonGetValue(content, "material");
    if (!mat.empty()) st.candidateMaterial = ParseCandidateMaterial(std::wstring(mat.begin(), mat.end()));
  }
  {
    std::string lay = isYaml ? SimpleYamlGetValue(content, "layout")
                              : SimpleJsonGetValue(content, "layout");
    if (!lay.empty()) st.candidateLayoutVariant = ParseCandidateLayoutVariant(std::wstring(lay.begin(), lay.end()));
  }
  {
    std::string fontSize = isYaml ? SimpleYamlGetValue(content, "font_size")
                                  : SimpleJsonGetValue(content, "font_size");
    if (!fontSize.empty()) {
      const int parsed = ParseIntStr(fontSize, -1);
      if (parsed > 0) st.candidateFontSize = std::clamp(static_cast<UINT>(parsed), 14u, 28u);
    }
  }
  {
    std::string fontFile = isYaml ? SimpleYamlGetValue(content, "font_file")
                                  : SimpleJsonGetValue(content, "font_file");
    if (!fontFile.empty()) {
      st.candidateFontFile = std::wstring(fontFile.begin(), fontFile.end());
    }
  }

  // Color overrides
  ApplySkinColor(content, "window_bg", isYaml, st.skinWindowBg);
  ApplySkinColor(content, "window_bg_to", isYaml, st.skinWindowBgTo);
  ApplySkinColor(content, "header_bg", isYaml, st.skinHeaderBg);
  ApplySkinColor(content, "header_bg_to", isYaml, st.skinHeaderBgTo);
  ApplySkinColor(content, "border", isYaml, st.skinBorder);
  ApplySkinColor(content, "divider", isYaml, st.skinDivider);
  ApplySkinColor(content, "text", isYaml, st.skinText);
  ApplySkinColor(content, "muted_text", isYaml, st.skinMutedText);
  ApplySkinColor(content, "badge_bg", isYaml, st.skinBadgeBg);
  ApplySkinColor(content, "badge_border", isYaml, st.skinBadgeBorder);
  ApplySkinColor(content, "badge_text", isYaml, st.skinBadgeText);
  ApplySkinColor(content, "hover_bg", isYaml, st.skinHoverBg);
  ApplySkinColor(content, "hover_border", isYaml, st.skinHoverBorder);
  ApplySkinColor(content, "item_bg", isYaml, st.skinItemBg);
  ApplySkinColor(content, "item_border", isYaml, st.skinItemBorder);
  ApplySkinColor(content, "selected_bg", isYaml, st.skinSelectedBg);
  ApplySkinColor(content, "selected_border", isYaml, st.skinSelectedBorder);
  ApplySkinColor(content, "pressed_bg", isYaml, st.skinPressedBg);
  ApplySkinColor(content, "pressed_border", isYaml, st.skinPressedBorder);
  ApplySkinColor(content, "selected_text", isYaml, st.skinSelectedText);
  ApplySkinColor(content, "selected_muted_text", isYaml, st.skinSelectedMutedText);
  ApplySkinColor(content, "chip_bg", isYaml, st.skinChipBg);
  ApplySkinColor(content, "chip_border", isYaml, st.skinChipBorder);
  ApplySkinColor(content, "chip_text", isYaml, st.skinChipText);
  ApplySkinColor(content, "chip_active_bg", isYaml, st.skinChipActiveBg);
  ApplySkinColor(content, "chip_active_border", isYaml, st.skinChipActiveBorder);
  ApplySkinColor(content, "chip_active_text", isYaml, st.skinChipActiveText);
  ApplySkinColor(content, "selected_outline", isYaml, st.skinSelectedOutline);
  ApplySkinInt(content, "selected_accent_width", isYaml, st.skinSelectedAccentWidth);
  ApplySkinFloat(content, "selected_ring_opacity", isYaml, st.skinSelectedRingOpacity);
  {
    std::string selectedIndicator =
        isYaml ? SimpleYamlGetValue(content, "selected_indicator")
               : SimpleJsonGetValue(content, "selected_indicator");
    if (!selectedIndicator.empty()) {
      std::transform(selectedIndicator.begin(), selectedIndicator.end(), selectedIndicator.begin(),
                     [](unsigned char ch) { return static_cast<char>(std::tolower(ch)); });
      if (selectedIndicator == "left_bar" || selectedIndicator == "bottom_bar" ||
          selectedIndicator == "outline" || selectedIndicator == "none") {
        st.skinSelectedIndicator =
            std::wstring(selectedIndicator.begin(), selectedIndicator.end());
      }
    }
  }

  // Opacity / size overrides
  ApplySkinFloat(content, "border_opacity", isYaml, st.skinBorderOpacity);
  ApplySkinFloat(content, "divider_opacity", isYaml, st.skinDividerOpacity);
  ApplySkinFloat(content, "shadow_opacity", isYaml, st.skinShadowOpacity);
  ApplySkinInt(content, "shadow_size", isYaml, st.skinShadowSize);
  ApplySkinBool(content, "shadow_enabled", isYaml, st.skinShadowEnabled);
  ApplySkinInt(content, "font_weight", isYaml, st.skinFontWeight);
  ApplySkinInt(content, "selected_font_weight", isYaml, st.skinSelectedFontWeight);
  ApplySkinInt(content, "label_font_weight", isYaml, st.skinLabelFontWeight);
  ApplySkinInt(content, "chip_font_weight", isYaml, st.skinChipFontWeight);
  ApplySkinInt(content, "corner_radius", isYaml, st.skinCornerRadius);
  ApplySkinInt(content, "header_corner_radius", isYaml, st.skinHeaderCornerRadius);
  ApplySkinInt(content, "row_corner_radius", isYaml, st.skinRowCornerRadius);
  ApplySkinInt(content, "badge_corner_radius", isYaml, st.skinBadgeCornerRadius);

  // Layout metric overrides (all ints; negative values disable override).
  ApplySkinInt(content, "outer_pad_x", isYaml, st.skinOuterPadX);
  ApplySkinInt(content, "outer_pad_y", isYaml, st.skinOuterPadY);
  ApplySkinInt(content, "header_pad_x", isYaml, st.skinHeaderPadX);
  ApplySkinInt(content, "header_pad_y", isYaml, st.skinHeaderPadY);
  ApplySkinInt(content, "header_gap", isYaml, st.skinHeaderGap);
  ApplySkinInt(content, "item_gap", isYaml, st.skinItemGap);
  ApplySkinInt(content, "item_pad_x", isYaml, st.skinItemPadX);
  ApplySkinInt(content, "item_pad_y", isYaml, st.skinItemPadY);
  ApplySkinInt(content, "label_width", isYaml, st.skinLabelWidth);
  ApplySkinInt(content, "label_gap", isYaml, st.skinLabelGap);
  ApplySkinInt(content, "comment_gap", isYaml, st.skinCommentGap);
  ApplySkinInt(content, "min_width", isYaml, st.skinMinWidth);
  ApplySkinInt(content, "preferred_width", isYaml, st.skinPreferredWidth);
  ApplySkinInt(content, "max_width", isYaml, st.skinMaxWidth);
  ApplySkinInt(content, "min_horizontal_card_width", isYaml, st.skinMinHorizontalCardWidth);
  ApplySkinInt(content, "max_horizontal_card_width", isYaml, st.skinMaxHorizontalCardWidth);

  if (st.skinFontWeight >= 0) st.skinFontWeight = std::clamp(st.skinFontWeight, 300, 700);
  if (st.skinSelectedFontWeight >= 0)
    st.skinSelectedFontWeight = std::clamp(st.skinSelectedFontWeight, 400, 800);
  if (st.skinLabelFontWeight >= 0) st.skinLabelFontWeight = std::clamp(st.skinLabelFontWeight, 400, 800);
  if (st.skinChipFontWeight >= 0) st.skinChipFontWeight = std::clamp(st.skinChipFontWeight, 350, 700);
  if (st.skinSelectedAccentWidth >= 0) st.skinSelectedAccentWidth = std::clamp(st.skinSelectedAccentWidth, 0, 8);
  if (st.skinSelectedRingOpacity >= 0.0f) st.skinSelectedRingOpacity = std::clamp(st.skinSelectedRingOpacity, 0.0f, 1.0f);

  st.skinLoaded = true;
}

void LoadSkinFile(SrfConfig& config) {
  LoadSkinFileFromConfigDir(config, ResolveSrfConfigPath().parent_path());
}

std::filesystem::path ResolveSrfConfigPath() {
  EnsureConfigWatcherStarted();
  ConfigCache& cache = GetConfigCache();
  std::lock_guard<std::mutex> lock(cache.mutex);
  return cache.path;
}

SrfConfig LoadSrfConfig() {
  EnsureConfigWatcherStarted();
  ConfigCache& cache = GetConfigCache();
  std::lock_guard<std::mutex> lock(cache.mutex);
  return cache.config;
}

uint64_t GetSrfConfigVersion() {
  EnsureConfigWatcherStarted();
  return GetConfigVersionAtomic().load(std::memory_order_acquire);
}
