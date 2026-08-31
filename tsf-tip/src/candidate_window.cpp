#include "candidate_window.h"

#include "candidate_layout_policy.h"

#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cwctype>
#include <filesystem>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

#include <d2d1.h>
#include <dwrite.h>
#include <dwrite_2.h>
#include <dwrite_3.h>
#include <gdiplus.h>
#include <wrl/client.h>
#include <windowsx.h>

/// 由 dllmain.cpp 提供，返回 DLL 自身的 HMODULE（非宿主 EXE）。
extern HMODULE SrfTip_GetDllModule();

extern void SrfTsfDiagnosticLog(const wchar_t* tag, const wchar_t* msg);
extern void SrfTsfPerfLog(const wchar_t* tag, const wchar_t* msg);

namespace {

/// 返回 DLL 模块句柄；若不可用则回退到宿主 EXE。
HINSTANCE DllOrFallbackInstance() {
  HMODULE m = SrfTip_GetDllModule();
  return m ? m : GetModuleHandleW(nullptr);
}

constexpr wchar_t kCandidateWndClass[] = L"SRF_TSF_Candidate_Window";
constexpr UINT_PTR kCandidatePaintTimerId = 73;
constexpr UINT_PTR kCandidateHorizontalShrinkTimerId = 74;
constexpr UINT_PTR kCandidateEnvironmentRefreshTimerId = 76;
constexpr UINT_PTR kCandidateAnimationTimerId = 77;
constexpr UINT_PTR kCandidatePendingIndicatorTimerId = 78;
constexpr ULONGLONG kCandidateMinImmediatePaintIntervalMs = 8;
constexpr UINT kCandidateHorizontalShrinkDelayMs = 96;
constexpr UINT kCandidateEnvironmentRefreshDelayMs = 80;
constexpr UINT kCandidateEnvironmentPollMs = 500;
constexpr ULONGLONG kCandidateStyleDeferMs = 30;
constexpr UINT kCandidateAnimationFrameMs = 15;
constexpr UINT kCandidatePendingIndicatorDelayMs = 80;
constexpr int kCandidateShowAnimationMs = 90;
constexpr int kCandidateSelectionAnimationMs = 80;
constexpr int kCandidateHoverAnimationMs = 70;
constexpr int kCandidatePressAnimationMs = 36;
constexpr int kCandidatePageAnimationMs = 110;
constexpr int kHorizontalPageBadgeMinWidth = 44;
constexpr int kHorizontalPageBadgeGap = 6;
constexpr int kHorizontalPageBadgePaddingY = 4;
constexpr wchar_t kDefaultCandidateFontFace[] = L"Microsoft YaHei";
constexpr int kPinMenuCommandNone = kSrfCandidateWindowMenuNone;
constexpr int kPinMenuCommandPin = kSrfCandidateWindowMenuPin;
constexpr int kPinMenuCommandUnpin = kSrfCandidateWindowMenuUnpin;
constexpr int kPinMenuCommandRemove = kSrfCandidateWindowMenuRemoveUserPhrase;
constexpr int kPinMenuCommandBlock = kSrfCandidateWindowMenuBlockPhrase;
constexpr int kPinMenuCommandSource = kSrfCandidateWindowMenuSource;

enum class CandidateWindowUpdateKind {
  Selection,
  Page,
  Content,
  Layout,
  Style,
};

std::wstring CandidatePageIndicatorText(UINT pageIndex, UINT totalPages) {
  const UINT safeTotalPages = std::max(1u, totalPages);
  const UINT safePageIndex = std::min(pageIndex, safeTotalPages - 1);
  return std::to_wstring(safePageIndex + 1) + L"/" +
         std::to_wstring(safeTotalPages);
}

float LinearAnimationProgress(ULONGLONG now, ULONGLONG start, int durationMs) {
  if (durationMs <= 0 || now <= start) return durationMs <= 0 ? 1.0f : 0.0f;
  return std::clamp(static_cast<float>(now - start) / static_cast<float>(durationMs), 0.0f, 1.0f);
}

float EaseOutCubic(float progress) {
  const float inverse = 1.0f - std::clamp(progress, 0.0f, 1.0f);
  return 1.0f - inverse * inverse * inverse;
}

float IndexTransitionWeight(size_t index, int from, int to, float progress) {
  if (from == to) return static_cast<int>(index) == to ? 1.0f : 0.0f;
  if (static_cast<int>(index) == from) return 1.0f - progress;
  if (static_cast<int>(index) == to) return progress;
  return 0.0f;
}

RECT InterpolateRect(const RECT& from, const RECT& to, float progress) {
  auto interpolate = [progress](LONG a, LONG b) {
    return static_cast<LONG>(
        std::lround(static_cast<double>(a) + static_cast<double>(b - a) * progress));
  };
  return {interpolate(from.left, to.left), interpolate(from.top, to.top),
          interpolate(from.right, to.right), interpolate(from.bottom, to.bottom)};
}

bool ShouldDeferImmediatePaint(CandidateWindowUpdateKind kind, bool windowVisible,
                               ULONGLONG lastShowTick, ULONGLONG showTick) {
  if (!windowVisible || lastShowTick == 0) return false;
  const ULONGLONG elapsed = showTick - lastShowTick;
  if ((kind == CandidateWindowUpdateKind::Content ||
       kind == CandidateWindowUpdateKind::Layout) &&
      elapsed < kCandidateMinImmediatePaintIntervalMs) {
    return true;
  }
  if (kind == CandidateWindowUpdateKind::Style && elapsed < kCandidateStyleDeferMs) return true;
  return false;
}

size_t CandidateItemPaintStateSlot(bool selected, bool hot, bool pressed) {
  if (pressed) return 3;
  if (hot) return 2;
  if (selected) return 1;
  return 0;
}

bool RectVectorEquals(const std::vector<RECT>& a, const std::vector<RECT>& b) {
  if (a.size() != b.size()) return false;
  for (size_t i = 0; i < a.size(); ++i) {
    if (!EqualRect(&a[i], &b[i])) return false;
  }
  return true;
}

#include "candidate_window_parts/candidate_window_text_renderer.ipp"

}  // namespace

SIZE MeasureSingleLine(HDC hdc, HFONT font, const std::wstring& text, UINT dpiOverride = 0) {
  if (!hdc || !font || text.empty()) return {};
  SIZE dwriteSize = {};
  if (GetDirectTextRenderer().MeasureSingleLine(hdc, font, text, &dwriteSize, dpiOverride)) {
    return dwriteSize;
  }
  font = ResolveTextFont(font, text);
  HGDIOBJ oldFont = SelectObject(hdc, font);
  SIZE size = {};
  GetTextExtentPoint32W(hdc, text.c_str(), static_cast<int>(text.size()), &size);
  if (oldFont) SelectObject(hdc, oldFont);
  return size;
}

void CandidateItemPaintCache::Release() {
  if (memDc && oldBitmap) {
    SelectObject(memDc, oldBitmap);
    oldBitmap = nullptr;
  }
  DeleteGdiObject(bitmap);
  bitmap = nullptr;
  if (memDc) {
    DeleteDC(memDc);
    memDc = nullptr;
  }
  w = 0;
  h = 0;
  itemIndex = SIZE_MAX;
  selected = false;
  hot = false;
  pressed = false;
  valid = false;
}

bool CandidateItemPaintCache::Ensure(HDC refDc, int width, int height) {
  if (width <= 0 || height <= 0) return false;
  if (memDc && bitmap && w == width && h == height) return true;
  Release();
  memDc = CreateCompatibleDC(refDc);
  if (!memDc) return false;
  bitmap = CreateCompatibleBitmap(refDc, width, height);
  if (!bitmap) {
    DeleteDC(memDc);
    memDc = nullptr;
    return false;
  }
  oldBitmap = static_cast<HBITMAP>(SelectObject(memDc, bitmap));
  w = width;
  h = height;
  return true;
}

SIZE CCandidateWindow::MeasureSingleLineCached(HDC hdc, HFONT font, const std::wstring& text) {
  if (!hdc || !font || text.empty()) return {};
  MeasuredTextCache* cache = nullptr;
  if (font == m_titleFont) {
    cache = &m_titleSizeCache;
  } else if (font == m_metaFont) {
    cache = &m_pageTextSizeCache;
  }
  if (cache && cache->Matches(font, m_dpi, text)) {
    return cache->size;
  }
  SIZE size = MeasureSingleLine(hdc, font, text, m_dpi);
  if (cache) {
    cache->text = text;
    cache->font = font;
    cache->dpi = m_dpi;
    cache->size = size;
  }
  return size;
}

SIZE CCandidateWindow::MeasureLabelCached(HDC hdc, const std::wstring& text, size_t index) {
  if (!hdc || !m_labelFont || text.empty()) return {};
  if (m_labelSizeCache.size() <= index) m_labelSizeCache.resize(index + 1);
  MeasuredTextCache& cache = m_labelSizeCache[index];
  if (cache.Matches(m_labelFont, m_dpi, text)) {
    return cache.size;
  }
  SIZE size = MeasureSingleLine(hdc, m_labelFont, text, m_dpi);
  cache.text = text;
  cache.font = m_labelFont;
  cache.dpi = m_dpi;
  cache.size = size;
  return size;
}

void CCandidateWindow::ClearMeasuredTextCaches() {
  m_titleSizeCache.Reset();
  m_pageTextSizeCache.Reset();
  m_labelSizeCache.clear();
}

UINT CurrentDpi(HWND hwnd) {
  if (hwnd) {
    const UINT dpi = GetDpiForWindow(hwnd);
    if (dpi != 0) return dpi;
  }
  const UINT dpi = GetDpiForSystem();
  return dpi == 0 ? 96u : dpi;
}

std::wstring AbbreviateCandidateForDisplay(const std::wstring& text, UINT maxChars) {
  if (text.empty()) return {};
  std::wstring compact;
  compact.reserve(text.size());
  bool prevSpace = false;
  for (wchar_t ch : text) {
    if (ch == L'\r' || ch == L'\n' || ch == L'\t') ch = L' ';
    if (std::iswspace(static_cast<wint_t>(ch))) {
      if (compact.empty() || prevSpace) continue;
      compact.push_back(L' ');
      prevSpace = true;
      continue;
    }
    compact.push_back(ch);
    prevSpace = false;
  }
  while (!compact.empty() && compact.back() == L' ') compact.pop_back();
  // rq/sj values are atomic. Truncating one character makes a date or time
  // unusable, so preserve canonical values and let the measured layout widen.
  const auto allDigits = [&](size_t start, size_t count) {
    if (start + count > compact.size()) return false;
    for (size_t i = start; i < start + count; ++i) {
      if (compact[i] < L'0' || compact[i] > L'9') return false;
    }
    return true;
  };
  const bool isoDate = compact.size() >= 10 && allDigits(0, 4) && compact[4] == L'-' &&
                       allDigits(5, 2) && compact[7] == L'-' && allDigits(8, 2);
  const bool clockTime = compact.size() >= 8 && allDigits(0, 2) && compact[2] == L':' &&
                         allDigits(3, 2) && compact[5] == L':' && allDigits(6, 2);
  const bool isoDateTime = compact.size() >= 19 && isoDate && compact[10] == L' ' &&
                           allDigits(11, 2) && compact[13] == L':' && allDigits(14, 2) &&
                           compact[16] == L':' && allDigits(17, 2);
  if ((compact.size() == 10 && isoDate) || (compact.size() == 8 && clockTime) ||
      (compact.size() == 19 && isoDateTime)) {
    return compact;
  }
  if (compact.empty() || maxChars == 0 || compact.size() <= maxChars) return compact;
  if (maxChars == 1) return L"…";
  size_t keep = maxChars - 1;
  if (keep < compact.size() && keep > 0 && compact[keep - 1] >= 0xD800 &&
      compact[keep - 1] <= 0xDBFF && compact[keep] >= 0xDC00 && compact[keep] <= 0xDFFF) {
    --keep;
  }
  compact.resize(keep);
  compact.push_back(L'…');
  return compact;
}

std::wstring ClipboardCandidatePreviewForDisplay(const std::wstring& text, size_t maxLines) {
  const size_t kMaxLines = std::clamp<size_t>(maxLines, 1, 3);
  constexpr size_t kMaxCharsPerLine = 72;
  constexpr size_t kMaxTotalChars = 180;
  std::vector<std::wstring> lines;
  std::wstring current;
  bool pendingSpace = false;
  bool truncated = false;
  size_t totalChars = 0;

  auto pushCurrent = [&]() {
    while (!current.empty() && std::iswspace(static_cast<wint_t>(current.back()))) {
      current.pop_back();
    }
    if (!current.empty()) {
      lines.push_back(current);
      totalChars += current.size();
    }
    current.clear();
    pendingSpace = false;
  };

  for (wchar_t ch : text) {
    if (ch == L'\r') continue;
    if (ch == L'\n') {
      pushCurrent();
      if (lines.size() >= kMaxLines) {
        truncated = true;
        break;
      }
      continue;
    }
    if (ch == L'\t' || std::iswspace(static_cast<wint_t>(ch))) {
      if (!current.empty()) pendingSpace = true;
      continue;
    }
    if (pendingSpace && !current.empty()) current.push_back(L' ');
    pendingSpace = false;
    current.push_back(ch);
    if (current.size() >= kMaxCharsPerLine ||
        totalChars + current.size() >= kMaxTotalChars) {
      truncated = true;
      pushCurrent();
      if (lines.size() >= kMaxLines) break;
    }
  }
  if (lines.size() < kMaxLines) pushCurrent();

  if (lines.empty()) return {};
  if (truncated && !lines.empty()) {
    std::wstring& last = lines.back();
    if (!last.empty()) {
      last.back() = L'\u2026';
    } else {
      last = L"\u2026";
    }
  }
  std::wstring out;
  for (size_t i = 0; i < lines.size(); ++i) {
    if (i > 0) out.push_back(L'\n');
    out += lines[i];
  }
  return out;
}

bool HasClipboardCandidateItems(const std::vector<bool>& values) {
  return std::any_of(values.begin(), values.end(), [](bool value) { return value; });
}

ClipboardCommentParts SplitClipboardComment(const std::wstring& comment) {
  const size_t tab = comment.find(L'\t');
  if (tab == std::wstring::npos) return {comment, {}};
  return {comment.substr(0, tab), comment.substr(tab + 1)};
}

RECT WorkAreaForAnchor(const RECT* anchorRect) {
  RECT work = {0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)};
  HMONITOR monitor = anchorRect ? MonitorFromRect(anchorRect, MONITOR_DEFAULTTONEAREST) : nullptr;
  if (!monitor) {
    POINT pt = {};
    if (GetCursorPos(&pt)) monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
  }
  MONITORINFO mi = {};
  mi.cbSize = sizeof(mi);
  if (monitor && GetMonitorInfoW(monitor, &mi)) work = mi.rcWork;
  return work;
}

RECT MonitorAreaForAnchor(const RECT* anchorRect) {
  RECT area = {0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)};
  HMONITOR monitor = anchorRect ? MonitorFromRect(anchorRect, MONITOR_DEFAULTTONEAREST) : nullptr;
  if (!monitor) {
    POINT pt = {};
    if (GetCursorPos(&pt)) monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
  }
  MONITORINFO mi = {};
  mi.cbSize = sizeof(mi);
  if (monitor && GetMonitorInfoW(monitor, &mi)) area = mi.rcMonitor;
  return area;
}

RECT PlacementAreaForAnchor(const RECT* anchorRect, bool fullscreenOverlay) {
  if (anchorRect && fullscreenOverlay) {
    return MonitorAreaForAnchor(anchorRect);
  }
  return WorkAreaForAnchor(anchorRect);
}

bool SidePlaceCandidateWindow(bool toRight, const RECT& anchor, const SIZE& size, const RECT& work,
                              int screenMargin, int gap, int* outX, int* outY) {
  const int minX = static_cast<int>(work.left) + screenMargin;
  const int maxX = work.right - screenMargin - size.cx;
  const int minY = static_cast<int>(work.top) + screenMargin;
  const int maxY = work.bottom - screenMargin - size.cy;
  if (minX > maxX || minY > maxY) return false;

  const int x = toRight ? (anchor.right + gap) : (anchor.left - gap - size.cx);
  if (x < minX || x > maxX) return false;

  *outX = x;
  *outY = std::clamp(static_cast<int>(anchor.top), minY, maxY);
  return true;
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

struct GdiplusRuntime {
  GdiplusRuntime() {
    Gdiplus::GdiplusStartupInput input;
    ready = Gdiplus::GdiplusStartup(&token, &input, nullptr) == Gdiplus::Ok;
  }
  ~GdiplusRuntime() {
    if (ready) Gdiplus::GdiplusShutdown(token);
  }
  ULONG_PTR token = 0;
  bool ready = false;
};

GdiplusRuntime*& GdiplusRuntimeStorage() {
  static GdiplusRuntime* runtime = nullptr;
  return runtime;
}

std::mutex& GdiplusRuntimeStorageMutex() {
  static std::mutex mutex;
  return mutex;
}

GdiplusRuntime& GetGdiplusRuntime() {
  std::lock_guard<std::mutex> lock(GdiplusRuntimeStorageMutex());
  GdiplusRuntime*& runtime = GdiplusRuntimeStorage();
  if (!runtime) runtime = new GdiplusRuntime();
  return *runtime;
}

void ShutdownGdiplusRuntime() {
  std::lock_guard<std::mutex> lock(GdiplusRuntimeStorageMutex());
  GdiplusRuntime*& runtime = GdiplusRuntimeStorage();
  delete runtime;
  runtime = nullptr;
}

void ShutdownCandidateWindowRendering() {
  ShutdownDirectTextRenderer();
  ShutdownGdiplusRuntime();
}

std::vector<std::filesystem::path> CandidateFontSearchRoots() {
  std::vector<std::filesystem::path> roots;
  const auto moduleDir = ModuleDirFromAddress(&BuildCandidatePageLayoutMetrics);
  if (!moduleDir.empty()) {
    std::filesystem::path current = moduleDir;
    for (int depth = 0; depth <= 4 && !current.empty(); ++depth) {
      if (std::find(roots.begin(), roots.end(), current) == roots.end()) roots.push_back(current);
      if (!current.has_parent_path()) break;
      const auto parent = current.parent_path();
      if (parent == current) break;
      current = parent;
    }
  }
  return roots;
}

bool IsAllowedLocalFontPath(const std::filesystem::path& path) {
  if (path.empty()) return false;
  const std::wstring raw = path.wstring();
  // 拒绝 UNC/网络路径，避免不可达共享盘阻塞输入线程。
  if (raw.rfind(L"\\\\", 0) == 0) return false;
  if (!path.is_absolute()) return true;
  const std::wstring root = path.root_name().wstring();
  return root.size() >= 2 && root[1] == L':';
}

std::filesystem::path ResolveBundledHarmonySansFontPath();

bool IsBundledHarmonySansFamily(const std::wstring& value) {
  std::wstring normalized = value;
  std::transform(normalized.begin(), normalized.end(), normalized.begin(),
                 [](wchar_t ch) { return static_cast<wchar_t>(towlower(ch)); });
  normalized.erase(std::remove_if(normalized.begin(), normalized.end(),
                                  [](wchar_t ch) {
                                    return ch == L' ' || ch == L'-' || ch == L'_';
                                  }),
                   normalized.end());
  return normalized == L"harmonyossans" || normalized == L"harmonyossanssc";
}

std::filesystem::path ResolveCandidateFontPath(const std::wstring& configuredFile) {
  if (configuredFile.empty()) return {};
  if (IsBundledHarmonySansFamily(configuredFile)) return ResolveBundledHarmonySansFontPath();
  const std::filesystem::path raw(configuredFile);
  if (raw.is_absolute() && !IsAllowedLocalFontPath(raw)) return {};
  std::error_code ec;
  if (raw.is_absolute() && std::filesystem::exists(raw, ec) && !ec) return raw;

  for (const auto& root : CandidateFontSearchRoots()) {
    const auto direct = root / raw;
    if (IsAllowedLocalFontPath(direct) && std::filesystem::exists(direct, ec) && !ec) return direct;
    const auto font1 = root / L"font1" / raw.filename();
    ec.clear();
    if (IsAllowedLocalFontPath(font1) && std::filesystem::exists(font1, ec) && !ec) return font1;
  }
  return {};
}

std::filesystem::path ResolveBundledHarmonySansFontPath() {
  static const std::filesystem::path candidates[] = {
      std::filesystem::path(L"HarmonyOS Sans") / L"HarmonyOS_Sans_SC.ttf",
      std::filesystem::path(L"font1") / L"HarmonyOS_Sans_SC.ttf",
      std::filesystem::path(L"font1") / L"HarmonyOS Sans" / L"HarmonyOS_Sans_SC.ttf",
  };
  std::error_code ec;
  for (const auto& root : CandidateFontSearchRoots()) {
    for (const auto& rel : candidates) {
      const auto path = root / rel;
      if (IsAllowedLocalFontPath(path) && std::filesystem::exists(path, ec) && !ec) {
        return path;
      }
      ec.clear();
    }
  }
  return {};
}

bool LooksLikeFontFileConfig(const std::wstring& configuredFile) {
  if (configuredFile.empty()) return false;
  if (configuredFile.find(L'\\') != std::wstring::npos ||
      configuredFile.find(L'/') != std::wstring::npos) {
    return true;
  }
  const std::filesystem::path raw(configuredFile);
  std::wstring ext = raw.extension().wstring();
  std::transform(ext.begin(), ext.end(), ext.begin(),
                 [](wchar_t ch) { return static_cast<wchar_t>(towlower(ch)); });
  return ext == L".ttf" || ext == L".otf" || ext == L".ttc";
}

std::wstring FontCacheKey(const std::filesystem::path& path) {
  std::wstring key = path.wstring();
  std::transform(key.begin(), key.end(), key.begin(),
                 [](wchar_t ch) { return static_cast<wchar_t>(towlower(ch)); });
  return key;
}

std::wstring ReadFontFamilyName(const std::filesystem::path& fontPath) {
  if (fontPath.empty() || !GetGdiplusRuntime().ready) return {};
  Gdiplus::PrivateFontCollection collection;
  if (collection.AddFontFile(fontPath.c_str()) != Gdiplus::Ok) return {};
  const int familyCount = collection.GetFamilyCount();
  if (familyCount <= 0) return {};
  std::vector<Gdiplus::FontFamily> families(static_cast<size_t>(familyCount));
  int found = 0;
  if (collection.GetFamilies(familyCount, families.data(), &found) != Gdiplus::Ok || found <= 0) {
    return {};
  }
  WCHAR familyName[LF_FACESIZE] = {};
  if (families[0].GetFamilyName(familyName) != Gdiplus::Ok) return {};
  return familyName;
}

struct PreparedFontEntry {
  std::wstring faceName;
  bool registered = false;
  std::filesystem::file_time_type writeTime = {};
  uintmax_t fileSize = 0;
};

std::mutex g_preparedFontMutex;
std::unordered_map<std::wstring, PreparedFontEntry> g_preparedFontCache;

PreparedFontEntry EnsurePreparedFontEntry(const std::filesystem::path& fontPath) {
  if (fontPath.empty()) return {};
  const std::wstring key = FontCacheKey(fontPath);
  std::error_code ec;
  const auto writeTime = std::filesystem::last_write_time(fontPath, ec);
  if (ec) return {};
  ec.clear();
  const uintmax_t fileSize = std::filesystem::file_size(fontPath, ec);
  if (ec) return {};
  {
    std::lock_guard<std::mutex> lock(g_preparedFontMutex);
    auto it = g_preparedFontCache.find(key);
    if (it != g_preparedFontCache.end() && it->second.writeTime == writeTime &&
        it->second.fileSize == fileSize) {
      return it->second;
    }
    if (it != g_preparedFontCache.end() && it->second.registered) {
      RemoveFontResourceExW(fontPath.c_str(), FR_PRIVATE, nullptr);
      g_preparedFontCache.erase(it);
    }
  }

  PreparedFontEntry entry;
  entry.writeTime = writeTime;
  entry.fileSize = fileSize;
  entry.faceName = ReadFontFamilyName(fontPath);
  entry.registered = AddFontResourceExW(fontPath.c_str(), FR_PRIVATE, nullptr) > 0;
  if (!entry.registered) entry.faceName.clear();
  std::lock_guard<std::mutex> lock(g_preparedFontMutex);
  auto [it, inserted] = g_preparedFontCache.emplace(key, entry);
  return it->second;
}

std::wstring ResolveConfiguredFontFaceName(const std::wstring& configuredFile,
                                           const std::filesystem::path& resolvedPath,
                                           const PreparedFontEntry& prepared) {
  if (!prepared.faceName.empty()) return prepared.faceName;
  if (!configuredFile.empty() && resolvedPath.empty() && !LooksLikeFontFileConfig(configuredFile)) {
    return configuredFile;
  }
  return kDefaultCandidateFontFace;
}

bool FontSupportsProbeText(HDC dc, HFONT font) {
  if (!dc || !font) return false;
  auto supports = [&](const wchar_t* text, int len) {
    if (!text || len <= 0) return false;
    std::vector<WORD> probeIndices(static_cast<size_t>(len), 0);
    const HGDIOBJ oldFont = SelectObject(dc, font);
    const DWORD result =
        GetGlyphIndicesW(dc, text, len, probeIndices.data(), GGI_MARK_NONEXISTING_GLYPHS);
    SelectObject(dc, oldFont);
    if (result == GDI_ERROR) return false;
    for (WORD index : probeIndices) {
      if (index == 0xFFFF) return false;
    }
    return true;
  };

  const wchar_t requiredProbe[] = L"\u5f00\u5fc3\u8f93\u5165\u6cd5\u4e2d\u6587ABC123";
  constexpr int kRequiredLen =
      static_cast<int>((sizeof(requiredProbe) / sizeof(wchar_t)) - 1);
  if (supports(requiredProbe, kRequiredLen)) return true;

  LOGFONTW lf = {};
  if (GetObjectW(font, sizeof(lf), &lf) && lf.lfFaceName[0] != L'\0') {
    const std::wstring face = lf.lfFaceName;
    if (face.find(L"KaiTi") != std::wstring::npos ||
        face.find(L"\u6977\u4f53") != std::wstring::npos ||
        face.find(L"FangSong") != std::wstring::npos ||
        face.find(L"\u4eff\u5b8b") != std::wstring::npos) {
      const wchar_t relaxedProbe[] = L"\u4e2d\u6587ABC123";
      constexpr int kRelaxedLen =
          static_cast<int>((sizeof(relaxedProbe) / sizeof(wchar_t)) - 1);
      return supports(relaxedProbe, kRelaxedLen);
    }
  }
  return false;
}

void DrawTextBlock(HDC hdc, HFONT font, COLORREF color, const std::wstring& text, RECT rect,
                   UINT format, UINT dpiOverride = 0);

bool PrimeCandidateRenderResources(HFONT titleFont, HFONT bodyFont, HFONT bodyStrongFont,
                                  HFONT metaFont, HFONT labelFont, HFONT chipFont) {
  if (!titleFont || !bodyFont || !bodyStrongFont || !metaFont || !labelFont || !chipFont) {
    return false;
  }

  HDC screenDc = GetDC(nullptr);
  if (!screenDc) return false;

  HDC memDc = CreateCompatibleDC(screenDc);
  HBITMAP bitmap = memDc ? CreateCompatibleBitmap(screenDc, 64, 64) : nullptr;
  HGDIOBJ oldBitmap = nullptr;
  if (memDc && bitmap) oldBitmap = SelectObject(memDc, bitmap);

  const auto release = [&]() {
    if (memDc && oldBitmap) SelectObject(memDc, oldBitmap);
    DeleteGdiObject(bitmap);
    DeleteDC(memDc);
    ReleaseDC(nullptr, screenDc);
  };

  const bool measuresOk =
      MeasureSingleLine(screenDc, titleFont, L"1 / 9").cx > 0 &&
      MeasureSingleLine(screenDc, bodyFont, L"\u5f00\u5fc3\u8f93\u5165\u6cd5").cx > 0 &&
      MeasureSingleLine(screenDc, metaFont, L"1 / 1").cx > 0 &&
      MeasureSingleLine(screenDc, labelFont, L"ni").cx > 0 &&
      MeasureSingleLine(screenDc, chipFont, L"ABC").cx > 0;

  bool drawOk = true;
  if (memDc && bitmap) {
    RECT rc = {0, 0, 64, 64};
    drawOk = GetDirectTextRenderer().DrawTextBlock(
        memDc, bodyStrongFont, RGB(0, 0, 0), L"\u5f00\u5fc3\u8f93\u5165\u6cd5", rc,
        DT_LEFT | DT_TOP | DT_SINGLELINE);
  }

  release();
  return measuresOk && drawOk;
}

#include "candidate_window_parts/candidate_window_theme.ipp"

#include "candidate_window_parts/candidate_window_layout.ipp"

int CandidateShadowMarginForStyle(const SrfUIStyle& style, UINT dpi) {
  const CandidateColors colors = ResolveColors(style);
  if (!colors.shadowEnabled || colors.shadowOpacity <= 0.001f || colors.shadowSize <= 0 ||
      style.themeMode == SrfThemeMode::HighContrast) {
    return 0;
  }
  return ScaleForDpi(std::clamp(colors.shadowSize, 0, 24), dpi);
}

int ResolveFontWeight(int value, int fallback, int minWeight = 300, int maxWeight = 800) {
  if (value <= 0) value = fallback;
  return std::clamp(value, minWeight, maxWeight);
}

int MeasureWrappedHeight(HDC hdc, HFONT font, const std::wstring& text, int width,
                         UINT dpiOverride = 0) {
  if (!hdc || !font || text.empty()) return 0;
  int dwriteHeight = 0;
  if (GetDirectTextRenderer().MeasureWrappedHeight(hdc, font, text, width, &dwriteHeight,
                                                   dpiOverride)) {
    return dwriteHeight;
  }
  font = ResolveTextFont(font, text);
  RECT rc = {0, 0, std::max(1, width), 0};
  HGDIOBJ oldFont = SelectObject(hdc, font);
  DrawTextW(hdc, text.c_str(), static_cast<int>(text.size()), &rc,
            DT_CALCRECT | DT_NOPREFIX | DT_WORDBREAK);
  if (oldFont) SelectObject(hdc, oldFont);
  return std::max(0, static_cast<int>(rc.bottom - rc.top));
}

void DrawTextBlock(HDC hdc, HFONT font, COLORREF color, const std::wstring& text, RECT rect,
                   UINT format, UINT dpiOverride) {
  if (!hdc || !font || text.empty()) return;
  if (GetDirectTextRenderer().DrawTextBlock(hdc, font, color, text, rect, format | DT_NOPREFIX,
                                            dpiOverride)) {
    return;
  }
  font = ResolveTextFont(font, text);
  HGDIOBJ oldFont = SelectObject(hdc, font);
  SetBkMode(hdc, TRANSPARENT);
  SetTextColor(hdc, color);
  DrawTextW(hdc, text.c_str(), static_cast<int>(text.size()), &rect, format | DT_NOPREFIX);
  if (oldFont) SelectObject(hdc, oldFont);
}

void DrawTextBlockWithOutline(HDC hdc, HFONT font, COLORREF color, COLORREF outline,
                              const std::wstring& text, RECT rect, UINT format) {
  if (!hdc || !font || text.empty()) return;
  bool dwriteComplete = true;
  for (int dy = -1; dy <= 1; ++dy) {
    for (int dx = -1; dx <= 1; ++dx) {
      if (dx == 0 && dy == 0) continue;
      RECT shadowRect = rect;
      OffsetRect(&shadowRect, dx, dy);
      dwriteComplete = GetDirectTextRenderer().DrawTextBlock(
          hdc, font, outline, text, shadowRect, format | DT_NOPREFIX) &&
                       dwriteComplete;
    }
  }
  dwriteComplete =
      GetDirectTextRenderer().DrawTextBlock(hdc, font, color, text, rect, format | DT_NOPREFIX) &&
      dwriteComplete;
  if (dwriteComplete) return;

  font = ResolveTextFont(font, text);
  HGDIOBJ oldFont = SelectObject(hdc, font);
  SetBkMode(hdc, TRANSPARENT);
  for (int dy = -1; dy <= 1; ++dy) {
    for (int dx = -1; dx <= 1; ++dx) {
      if (dx == 0 && dy == 0) continue;
      RECT shadowRect = rect;
      OffsetRect(&shadowRect, dx, dy);
      SetTextColor(hdc, outline);
      DrawTextW(hdc, text.c_str(), static_cast<int>(text.size()), &shadowRect, format | DT_NOPREFIX);
    }
  }
  SetTextColor(hdc, color);
  DrawTextW(hdc, text.c_str(), static_cast<int>(text.size()), &rect, format | DT_NOPREFIX);
  if (oldFont) SelectObject(hdc, oldFont);
}

void FillBorderedRoundRect(HDC hdc, const RECT& rect, COLORREF fill, COLORREF border, int radius,
                           int strokeWidth) {
  HBRUSH brush = CreateSolidBrush(fill);
  HPEN pen = CreatePen(PS_SOLID, std::max(1, strokeWidth), border);
  HGDIOBJ oldBrush = SelectObject(hdc, brush);
  HGDIOBJ oldPen = SelectObject(hdc, pen);
  RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
  if (oldPen) SelectObject(hdc, oldPen);
  if (oldBrush) SelectObject(hdc, oldBrush);
  DeleteGdiObject(pen);
  DeleteGdiObject(brush);
}

void FillSolidRect(HDC hdc, const RECT& rect, COLORREF color) {
  HBRUSH brush = CreateSolidBrush(color);
  FillRect(hdc, &rect, brush);
  DeleteGdiObject(brush);
}

void FillGradientRect(HDC hdc, const RECT& rect, COLORREF colorTop, COLORREF colorBottom) {
  TRIVERTEX vertex[2] = {};
  vertex[0].x = rect.left;
  vertex[0].y = rect.top;
  vertex[0].Red = GetRValue(colorTop) << 8;
  vertex[0].Green = GetGValue(colorTop) << 8;
  vertex[0].Blue = GetBValue(colorTop) << 8;
  vertex[0].Alpha = 0xff00;
  vertex[1].x = rect.right;
  vertex[1].y = rect.bottom;
  vertex[1].Red = GetRValue(colorBottom) << 8;
  vertex[1].Green = GetGValue(colorBottom) << 8;
  vertex[1].Blue = GetBValue(colorBottom) << 8;
  vertex[1].Alpha = 0xff00;
  GRADIENT_RECT gRect = {0, 1};
  GradientFill(hdc, vertex, 2, &gRect, 1, GRADIENT_FILL_RECT_V);
}

void FillBackground(HDC hdc, const RECT& rect, const CandidateColors& colors) {
  if (colors.windowBgTo != CLR_INVALID) {
    FillGradientRect(hdc, rect, colors.windowBg, colors.windowBgTo);
  } else {
    FillSolidRect(hdc, rect, colors.windowBg);
  }
}

void FillHeaderBackground(HDC hdc, const RECT& rect, const CandidateColors& colors) {
  if (colors.headerBgTo != CLR_INVALID) {
    FillGradientRect(hdc, rect, colors.headerBg, colors.headerBgTo);
  } else {
    FillSolidRect(hdc, rect, colors.headerBg);
  }
}

void DrawDivider(HDC hdc, int y, int left, int right, const CandidateColors& colors,
                 int strokeWidth) {
  if (colors.divider == CLR_INVALID) return;
  const COLORREF divColor = AlphaBlendColor(colors.divider, colors.windowBg, colors.dividerOpacity);
  HPEN pen = CreatePen(PS_SOLID, std::max(1, strokeWidth), divColor);
  HGDIOBJ oldPen = SelectObject(hdc, pen);
  MoveToEx(hdc, left, y, nullptr);
  LineTo(hdc, right, y);
  SelectObject(hdc, oldPen);
  DeleteGdiObject(pen);
}

// GDI brush/pen cache for high-frequency paint operations.
struct GdiCache {
  COLORREF brushColor = CLR_INVALID;
  HBRUSH brush = nullptr;
  COLORREF penColor = CLR_INVALID;
  int penWidth = 0;
  HPEN pen = nullptr;

  HBRUSH GetBrush(COLORREF color) {
    if (brush && brushColor == color) return brush;
    DeleteGdiObject(brush);
    brush = CreateSolidBrush(color);
    brushColor = color;
    return brush;
  }

  HPEN GetPen(COLORREF color, int width) {
    width = std::max(1, width);
    if (pen && penColor == color && penWidth == width) return pen;
    DeleteGdiObject(pen);
    pen = CreatePen(PS_SOLID, width, color);
    penColor = color;
    penWidth = width;
    return pen;
  }

  void Reset() {
    DeleteGdiObject(brush);
    DeleteGdiObject(pen);
    brush = nullptr;
    pen = nullptr;
    brushColor = CLR_INVALID;
    penColor = CLR_INVALID;
    penWidth = 0;
  }
};

GdiCache& GetGdiCache() {
  static GdiCache cache;
  return cache;
}

double SignedDistanceToRoundedRect(double x, double y, double left, double top, double right,
                                   double bottom, double radius) {
  const double cx = (left + right) * 0.5;
  const double cy = (top + bottom) * 0.5;
  const double hx = std::max(0.0, (right - left) * 0.5);
  const double hy = std::max(0.0, (bottom - top) * 0.5);
  const double r = std::clamp(radius, 0.0, std::min(hx, hy));
  const double qx = std::abs(x - cx) - (hx - r);
  const double qy = std::abs(y - cy) - (hy - r);
  const double ox = std::max(qx, 0.0);
  const double oy = std::max(qy, 0.0);
  const double outside = std::sqrt(ox * ox + oy * oy);
  const double inside = std::min(std::max(qx, qy), 0.0);
  return outside + inside - r;
}

struct SoftShadowCache {
  HDC memDc = nullptr;
  HBITMAP bitmap = nullptr;
  HBITMAP oldBitmap = nullptr;
  int bitmapW = 0;
  int bitmapH = 0;
  int rectW = 0;
  int rectH = 0;
  int radius = 0;
  int spread = 0;
  int maxAlpha = 0;

  ~SoftShadowCache() { Reset(); }

  void Reset() {
    if (memDc && oldBitmap) {
      SelectObject(memDc, oldBitmap);
      oldBitmap = nullptr;
    }
    DeleteGdiObject(bitmap);
    bitmap = nullptr;
    if (memDc) {
      DeleteDC(memDc);
      memDc = nullptr;
    }
    bitmapW = 0;
    bitmapH = 0;
    rectW = 0;
    rectH = 0;
    radius = 0;
    spread = 0;
    maxAlpha = 0;
  }

  bool Ensure(int nextRectW, int nextRectH, int nextRadius, int nextSpread,
              int nextMaxAlpha) {
    if (bitmap && memDc && rectW == nextRectW && rectH == nextRectH &&
        radius == nextRadius && spread == nextSpread && maxAlpha == nextMaxAlpha) {
      return true;
    }
    Reset();
    if (nextRectW <= 0 || nextRectH <= 0 || nextSpread <= 0 || nextMaxAlpha <= 0) return false;

    bitmapW = nextRectW + nextSpread * 2;
    bitmapH = nextRectH + nextSpread * 2;
    BITMAPINFO bmi = {};
    bmi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    bmi.bmiHeader.biWidth = bitmapW;
    bmi.bmiHeader.biHeight = -bitmapH;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;
    void* rawBits = nullptr;
    bitmap = CreateDIBSection(nullptr, &bmi, DIB_RGB_COLORS, &rawBits, nullptr, 0);
    if (!bitmap || !rawBits) {
      Reset();
      return false;
    }

    auto* pixels = static_cast<std::uint32_t*>(rawBits);
    const double left = static_cast<double>(nextSpread);
    const double top = static_cast<double>(nextSpread);
    const double right = left + static_cast<double>(nextRectW);
    const double bottom = top + static_cast<double>(nextRectH);
    const double spreadD = static_cast<double>(nextSpread);
    const double radiusD = static_cast<double>(nextRadius);
    for (int y = 0; y < bitmapH; ++y) {
      for (int x = 0; x < bitmapW; ++x) {
        const double distance =
            SignedDistanceToRoundedRect(static_cast<double>(x) + 0.5,
                                        static_cast<double>(y) + 0.5, left, top,
                                        right, bottom, radiusD);
        // Only the area outside the rounded rect is allowed to receive shadow
        // alpha. The candidate window currently paints into its client rect,
        // so giving the inside area alpha would darken the whole candidate bar.
        if (distance <= 0.0 || distance > spreadD) {
          pixels[y * bitmapW + x] = 0;
          continue;
        }
        const double t = std::clamp(1.0 - std::max(0.0, distance) / spreadD, 0.0, 1.0);
        const int alpha =
            std::clamp(static_cast<int>(std::lround(nextMaxAlpha * t * t)), 0, 255);
        pixels[y * bitmapW + x] = static_cast<std::uint32_t>(alpha) << 24;
      }
    }

    memDc = CreateCompatibleDC(nullptr);
    if (!memDc) {
      Reset();
      return false;
    }
    oldBitmap = static_cast<HBITMAP>(SelectObject(memDc, bitmap));
    rectW = nextRectW;
    rectH = nextRectH;
    radius = nextRadius;
    spread = nextSpread;
    maxAlpha = nextMaxAlpha;
    return true;
  }
};

SoftShadowCache& GetSoftShadowCache() {
  static SoftShadowCache cache;
  return cache;
}

void FillBorderedRoundRectCached(HDC hdc, const RECT& rect, COLORREF fill, COLORREF border,
                                 int radius, int strokeWidth) {
  GdiCache& cache = GetGdiCache();
  HGDIOBJ oldBrush = SelectObject(hdc, cache.GetBrush(fill));
  HGDIOBJ oldPen = SelectObject(hdc, cache.GetPen(border, strokeWidth));
  RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
  if (oldPen) SelectObject(hdc, oldPen);
  if (oldBrush) SelectObject(hdc, oldBrush);
}

void DrawRoundRectBorderCached(HDC hdc, const RECT& rect, COLORREF border, int radius,
                               int strokeWidth) {
  GdiCache& cache = GetGdiCache();
  HGDIOBJ oldBrush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
  HGDIOBJ oldPen = SelectObject(hdc, cache.GetPen(border, strokeWidth));
  RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
  if (oldPen) SelectObject(hdc, oldPen);
  if (oldBrush) SelectObject(hdc, oldBrush);
}

void FillBackgroundClippedToRoundRect(HDC hdc, const RECT& rect, const CandidateColors& colors,
                                      int radius) {
  if (radius <= 0) {
    FillBackground(hdc, rect, colors);
    return;
  }
  HRGN clip = CreateRoundRectRgn(rect.left, rect.top, rect.right + 1, rect.bottom + 1,
                                 radius, radius);
  if (!clip) {
    FillBackground(hdc, rect, colors);
    return;
  }
  const int saved = SaveDC(hdc);
  SelectClipRgn(hdc, clip);
  FillBackground(hdc, rect, colors);
  if (saved != 0) RestoreDC(hdc, saved);
  DeleteObject(clip);
}

void FillHeaderBackgroundClippedToRoundRect(HDC hdc, const RECT& rect,
                                            const CandidateColors& colors, int radius) {
  if (radius <= 0) {
    FillHeaderBackground(hdc, rect, colors);
    return;
  }
  HRGN clip = CreateRoundRectRgn(rect.left, rect.top, rect.right + 1, rect.bottom + 1,
                                 radius, radius);
  if (!clip) {
    FillHeaderBackground(hdc, rect, colors);
    return;
  }
  const int saved = SaveDC(hdc);
  SelectClipRgn(hdc, clip);
  FillHeaderBackground(hdc, rect, colors);
  if (saved != 0) RestoreDC(hdc, saved);
  DeleteObject(clip);
}

bool CandidateWindowResourceStyleEquals(const SrfUIStyle& a, const SrfUIStyle& b) {
  return a.candidateFontSize == b.candidateFontSize &&
         a.candidateScalePercent == b.candidateScalePercent &&
         a.candidateFontFile == b.candidateFontFile &&
         a.candidateFontWeight == b.candidateFontWeight &&
         a.candidateSelectedFontWeight == b.candidateSelectedFontWeight &&
         a.candidateLabelFontWeight == b.candidateLabelFontWeight &&
         a.candidateChipFontWeight == b.candidateChipFontWeight &&
         a.skinLoaded == b.skinLoaded &&
         a.skinFontWeight == b.skinFontWeight &&
         a.skinSelectedFontWeight == b.skinSelectedFontWeight &&
         a.skinLabelFontWeight == b.skinLabelFontWeight &&
         a.skinChipFontWeight == b.skinChipFontWeight;
}

bool CandidateWindowPaintStyleEquals(const SrfUIStyle& a, const SrfUIStyle& b) {
  return a.candidateMaterial == b.candidateMaterial && a.themeMode == b.themeMode &&
         a.skinLoaded == b.skinLoaded && a.skinWindowBg == b.skinWindowBg &&
         a.skinWindowBgTo == b.skinWindowBgTo && a.skinHeaderBg == b.skinHeaderBg &&
         a.skinHeaderBgTo == b.skinHeaderBgTo && a.skinBorder == b.skinBorder &&
         a.skinDivider == b.skinDivider && a.skinText == b.skinText &&
         a.skinMutedText == b.skinMutedText && a.skinBadgeBg == b.skinBadgeBg &&
         a.skinBadgeBorder == b.skinBadgeBorder && a.skinBadgeText == b.skinBadgeText &&
         a.skinHoverBg == b.skinHoverBg && a.skinHoverBorder == b.skinHoverBorder &&
         a.skinItemBg == b.skinItemBg && a.skinItemBorder == b.skinItemBorder &&
         a.skinSelectedBg == b.skinSelectedBg && a.skinSelectedBorder == b.skinSelectedBorder &&
         a.skinPressedBg == b.skinPressedBg && a.skinPressedBorder == b.skinPressedBorder &&
         a.skinSelectedText == b.skinSelectedText &&
         a.skinSelectedMutedText == b.skinSelectedMutedText && a.skinChipBg == b.skinChipBg &&
         a.skinChipBorder == b.skinChipBorder && a.skinChipText == b.skinChipText &&
         a.skinChipActiveBg == b.skinChipActiveBg &&
         a.skinChipActiveBorder == b.skinChipActiveBorder &&
         a.skinChipActiveText == b.skinChipActiveText &&
         a.skinSelectedOutline == b.skinSelectedOutline &&
         a.skinSelectedAccentWidth == b.skinSelectedAccentWidth &&
         a.skinSelectedRingOpacity == b.skinSelectedRingOpacity &&
         a.skinSelectedIndicator == b.skinSelectedIndicator &&
         a.skinBorderOpacity == b.skinBorderOpacity &&
         a.skinDividerOpacity == b.skinDividerOpacity &&
         a.skinShadowOpacity == b.skinShadowOpacity && a.skinShadowSize == b.skinShadowSize &&
         a.skinShadowEnabled == b.skinShadowEnabled &&
         a.skinAnimationsEnabled == b.skinAnimationsEnabled &&
         a.skinShowAnimationMs == b.skinShowAnimationMs &&
         a.skinSelectionAnimationMs == b.skinSelectionAnimationMs &&
         a.skinHoverAnimationMs == b.skinHoverAnimationMs &&
         a.skinPressAnimationMs == b.skinPressAnimationMs &&
         a.skinPageAnimationMs == b.skinPageAnimationMs &&
         a.skinCornerRadius == b.skinCornerRadius &&
         a.skinHeaderCornerRadius == b.skinHeaderCornerRadius &&
         a.skinRowCornerRadius == b.skinRowCornerRadius &&
         a.skinBadgeCornerRadius == b.skinBadgeCornerRadius;
}

bool CandidateWindowStyleEquals(const SrfUIStyle& a, const SrfUIStyle& b) {
  return a.inlinePreedit == b.inlinePreedit && a.enhancedPosition == b.enhancedPosition &&
         a.pagingOnScroll == b.pagingOnScroll &&
         a.candidateAbbreviateLength == b.candidateAbbreviateLength &&
         a.candidateFontSize == b.candidateFontSize && a.candidateOpacity == b.candidateOpacity &&
         a.candidateReduceMotion == b.candidateReduceMotion &&
         a.candidateFontFile == b.candidateFontFile &&
         a.candidateFontWeight == b.candidateFontWeight &&
         a.candidateSelectedFontWeight == b.candidateSelectedFontWeight &&
         a.candidateLabelFontWeight == b.candidateLabelFontWeight &&
         a.candidateChipFontWeight == b.candidateChipFontWeight &&
         a.candidateSkinFile == b.candidateSkinFile &&
         a.candidateHorizontal == b.candidateHorizontal &&
         a.candidatePageSize == b.candidatePageSize &&
         a.candidateHorizontalCount == b.candidateHorizontalCount &&
         a.candidateHorizontalCompact == b.candidateHorizontalCompact &&
         a.showCandidateReading == b.showCandidateReading &&
         a.showCandidateScore == b.showCandidateScore &&
         a.highlightTypoCandidates == b.highlightTypoCandidates &&
         a.showCandidateSource == b.showCandidateSource &&
         a.showModeInCandidateHeader == b.showModeInCandidateHeader &&
         a.candidateTopmost == b.candidateTopmost && a.candidateLeftClick == b.candidateLeftClick &&
         a.candidateRightClick == b.candidateRightClick && a.themeMode == b.themeMode &&
         a.candidateMaterial == b.candidateMaterial && a.candidateDensity == b.candidateDensity &&
         a.candidateLayoutVariant == b.candidateLayoutVariant &&
         a.candidateScalePercent == b.candidateScalePercent &&
         a.candidateOverlayAnchor == b.candidateOverlayAnchor &&
         a.candidateFullscreenPlacement == b.candidateFullscreenPlacement &&
         a.skinWindowBg == b.skinWindowBg && a.skinWindowBgTo == b.skinWindowBgTo &&
         a.skinHeaderBg == b.skinHeaderBg && a.skinHeaderBgTo == b.skinHeaderBgTo &&
         a.skinBorder == b.skinBorder && a.skinDivider == b.skinDivider &&
         a.skinText == b.skinText && a.skinMutedText == b.skinMutedText &&
         a.skinBadgeBg == b.skinBadgeBg && a.skinBadgeBorder == b.skinBadgeBorder &&
         a.skinBadgeText == b.skinBadgeText && a.skinHoverBg == b.skinHoverBg &&
         a.skinHoverBorder == b.skinHoverBorder && a.skinItemBg == b.skinItemBg &&
         a.skinItemBorder == b.skinItemBorder && a.skinSelectedBg == b.skinSelectedBg &&
         a.skinSelectedBorder == b.skinSelectedBorder && a.skinPressedBg == b.skinPressedBg &&
         a.skinPressedBorder == b.skinPressedBorder && a.skinSelectedText == b.skinSelectedText &&
         a.skinSelectedMutedText == b.skinSelectedMutedText && a.skinChipBg == b.skinChipBg &&
         a.skinChipBorder == b.skinChipBorder && a.skinChipText == b.skinChipText &&
         a.skinChipActiveBg == b.skinChipActiveBg &&
         a.skinChipActiveBorder == b.skinChipActiveBorder &&
         a.skinChipActiveText == b.skinChipActiveText &&
         a.skinSelectedOutline == b.skinSelectedOutline &&
         a.skinSelectedAccentWidth == b.skinSelectedAccentWidth &&
         a.skinSelectedRingOpacity == b.skinSelectedRingOpacity &&
         a.skinSelectedIndicator == b.skinSelectedIndicator &&
         a.skinBorderOpacity == b.skinBorderOpacity &&
         a.skinDividerOpacity == b.skinDividerOpacity &&
         a.skinShadowOpacity == b.skinShadowOpacity && a.skinShadowSize == b.skinShadowSize &&
         a.skinShadowEnabled == b.skinShadowEnabled &&
         a.skinAnimationsEnabled == b.skinAnimationsEnabled &&
         a.skinShowAnimationMs == b.skinShowAnimationMs &&
         a.skinSelectionAnimationMs == b.skinSelectionAnimationMs &&
         a.skinHoverAnimationMs == b.skinHoverAnimationMs &&
         a.skinPressAnimationMs == b.skinPressAnimationMs &&
         a.skinPageAnimationMs == b.skinPageAnimationMs && a.skinFontWeight == b.skinFontWeight &&
         a.skinSelectedFontWeight == b.skinSelectedFontWeight &&
         a.skinLabelFontWeight == b.skinLabelFontWeight &&
         a.skinChipFontWeight == b.skinChipFontWeight && a.skinCornerRadius == b.skinCornerRadius &&
         a.skinHeaderCornerRadius == b.skinHeaderCornerRadius &&
         a.skinRowCornerRadius == b.skinRowCornerRadius &&
         a.skinBadgeCornerRadius == b.skinBadgeCornerRadius && a.skinOuterPadX == b.skinOuterPadX &&
         a.skinOuterPadY == b.skinOuterPadY && a.skinHeaderPadX == b.skinHeaderPadX &&
         a.skinHeaderPadY == b.skinHeaderPadY && a.skinHeaderGap == b.skinHeaderGap &&
         a.skinItemGap == b.skinItemGap && a.skinItemPadX == b.skinItemPadX &&
         a.skinItemPadY == b.skinItemPadY && a.skinLabelWidth == b.skinLabelWidth &&
         a.skinLabelGap == b.skinLabelGap && a.skinCommentGap == b.skinCommentGap &&
         a.skinMinWidth == b.skinMinWidth && a.skinPreferredWidth == b.skinPreferredWidth &&
         a.skinMaxWidth == b.skinMaxWidth &&
         a.skinMinHorizontalCardWidth == b.skinMinHorizontalCardWidth &&
         a.skinMaxHorizontalCardWidth == b.skinMaxHorizontalCardWidth &&
         a.skinLoaded == b.skinLoaded;
}

bool CandidateWindowLayoutStyleEquals(const SrfUIStyle& a, const SrfUIStyle& b) {
  return a.candidateAbbreviateLength == b.candidateAbbreviateLength &&
         a.candidateFontSize == b.candidateFontSize &&
         a.candidateFontFile == b.candidateFontFile &&
         a.candidateFontWeight == b.candidateFontWeight &&
         a.candidateSelectedFontWeight == b.candidateSelectedFontWeight &&
         a.candidateLabelFontWeight == b.candidateLabelFontWeight &&
         a.candidateChipFontWeight == b.candidateChipFontWeight &&
         a.candidateSkinFile == b.candidateSkinFile &&
         a.candidateHorizontal == b.candidateHorizontal &&
         a.candidatePageSize == b.candidatePageSize &&
         a.candidateHorizontalCount == b.candidateHorizontalCount &&
         a.candidateHorizontalCompact == b.candidateHorizontalCompact &&
         a.showCandidateReading == b.showCandidateReading &&
         a.showCandidateScore == b.showCandidateScore &&
         a.highlightTypoCandidates == b.highlightTypoCandidates &&
         a.showCandidateSource == b.showCandidateSource &&
         a.showModeInCandidateHeader == b.showModeInCandidateHeader &&
         a.candidateLeftClick == b.candidateLeftClick &&
         a.candidateRightClick == b.candidateRightClick &&
         a.candidateDensity == b.candidateDensity &&
         a.candidateLayoutVariant == b.candidateLayoutVariant &&
         a.candidateScalePercent == b.candidateScalePercent &&
         a.candidateOverlayAnchor == b.candidateOverlayAnchor &&
         a.candidateFullscreenPlacement == b.candidateFullscreenPlacement &&
         a.skinFontWeight == b.skinFontWeight &&
         a.skinSelectedFontWeight == b.skinSelectedFontWeight &&
         a.skinLabelFontWeight == b.skinLabelFontWeight &&
         a.skinChipFontWeight == b.skinChipFontWeight &&
         a.skinOuterPadX == b.skinOuterPadX && a.skinOuterPadY == b.skinOuterPadY &&
         a.skinHeaderPadX == b.skinHeaderPadX && a.skinHeaderPadY == b.skinHeaderPadY &&
         a.skinHeaderGap == b.skinHeaderGap && a.skinItemGap == b.skinItemGap &&
         a.skinItemPadX == b.skinItemPadX && a.skinItemPadY == b.skinItemPadY &&
         a.skinLabelWidth == b.skinLabelWidth && a.skinLabelGap == b.skinLabelGap &&
         a.skinCommentGap == b.skinCommentGap && a.skinMinWidth == b.skinMinWidth &&
         a.skinPreferredWidth == b.skinPreferredWidth && a.skinMaxWidth == b.skinMaxWidth &&
         a.skinMinHorizontalCardWidth == b.skinMinHorizontalCardWidth &&
         a.skinMaxHorizontalCardWidth == b.skinMaxHorizontalCardWidth &&
         a.skinLoaded == b.skinLoaded;
}

UINT DpiForScreenRect(const RECT* rect) {
  auto systemDpi = []() -> UINT {
    const UINT dpi = GetDpiForSystem();
    return dpi == 0 ? 96u : dpi;
  };
  if (!rect) return systemDpi();
  HMONITOR mon = MonitorFromRect(rect, MONITOR_DEFAULTTONEAREST);
  using GetDpiForMonitorFn = HRESULT(WINAPI*)(HMONITOR, int, UINT*, UINT*);
  static GetDpiForMonitorFn pGetDpiForMonitor = nullptr;
  static bool resolved = false;
  if (!resolved) {
    resolved = true;
    HMODULE shcore = LoadLibraryW(L"shcore.dll");
    if (shcore) {
      pGetDpiForMonitor =
          reinterpret_cast<GetDpiForMonitorFn>(GetProcAddress(shcore, "GetDpiForMonitor"));
    }
  }
  UINT dpiX = 0, dpiY = 0;
  if (pGetDpiForMonitor && mon &&
      SUCCEEDED(pGetDpiForMonitor(mon, 0 /* MDT_EFFECTIVE_DPI */, &dpiX, &dpiY)) && dpiX != 0) {
    return dpiX;
  }
  return systemDpi();
}

CandidatePageLayoutMetrics BuildCandidatePageLayoutMetrics(const SrfUIStyle& style,
                                                           const RECT* anchorRect,
                                                           const std::vector<std::wstring>& items,
                                                           UINT dpi) {
  CandidatePageLayoutMetrics metrics = {};
  metrics.itemWidths.resize(items.size(), 0);
  if (items.empty()) return metrics;

  const UINT resolvedDpi = dpi == 0 ? DpiForScreenRect(anchorRect) : dpi;
  const UINT layoutDpi = static_cast<UINT>(std::clamp(
      MulDiv(static_cast<int>(resolvedDpi),
             static_cast<int>(std::clamp(style.candidateScalePercent, 50u, 200u)), 100),
      48, 1536));
  if (!style.candidateHorizontal) {
    metrics.pageStarts.push_back(0);
    const UINT pageSize = std::clamp(style.candidatePageSize, 3u, 10u);
    for (UINT i = pageSize; i < items.size(); i += pageSize) {
      metrics.pageStarts.push_back(i);
    }
    return metrics;
  }

  const int fullAreaWidth =
      ResolveHorizontalCardsAreaWidth(style, anchorRect, layoutDpi, false);
  const int firstPageVisible =
      ResolveHorizontalVisibleCountForItems(style, layoutDpi, fullAreaWidth, items, 0);
  const bool reservePageIndicator =
      firstPageVisible > 0 && static_cast<size_t>(firstPageVisible) < items.size();
  const int areaWidth = reservePageIndicator
                            ? ResolveHorizontalCardsAreaWidth(style, anchorRect, layoutDpi, true)
                            : fullAreaWidth;
  size_t start = 0;
  while (start < items.size()) {
    metrics.pageStarts.push_back(static_cast<UINT>(start));
    const int visible =
        ResolveHorizontalVisibleCountForItems(style, layoutDpi, areaWidth, items, start);
    const int cardWidth = ResolveHorizontalCardWidthForArea(style, layoutDpi, areaWidth, visible);
    for (int i = 0; i < visible && (start + static_cast<size_t>(i)) < items.size(); ++i) {
      metrics.itemWidths[start + static_cast<size_t>(i)] = cardWidth;
    }
    start += static_cast<size_t>(std::max(1, visible));
  }

  return metrics;
}

CCandidateWindow::~CCandidateWindow() { Destroy(); }

void CCandidateWindow::SetEvents(ICandidateWindowEvents* events) { m_events = events; }

void CCandidateWindow::SetGameOverlay(bool enabled, bool fullscreen, HWND targetHwnd) {
  if (targetHwnd) {
    HWND root = GetAncestor(targetHwnd, GA_ROOT);
    if (root) targetHwnd = root;
  }
  if (!enabled) {
    fullscreen = false;
    targetHwnd = nullptr;
  }
  if (m_gameOverlay == enabled && m_fullscreenOverlayPlacement == fullscreen &&
      m_overlayTargetHwnd == targetHwnd) {
    return;
  }

  const bool placementChanged = m_fullscreenOverlayPlacement != fullscreen;
  m_gameOverlay = enabled;
  m_fullscreenOverlayPlacement = fullscreen;
  m_overlayTargetHwnd = targetHwnd;
  m_overlayEnvironmentValid = false;
  m_overlayObservedMonitor = nullptr;
  m_overlayObservedTargetRect = {};
  m_overlayObservedDpi = 0;
  if (!m_gameOverlay) CancelEnvironmentRefresh();
  if (m_gameOverlay && m_hwnd && GetCapture() == m_hwnd) ReleaseCapture();
  if (m_gameOverlay) {
    CancelAnimations(true);
    m_hotIndex = -1;
    m_pressedIndex = -1;
    m_rightPressedIndex = -1;
    m_trackingMouse = false;
  }
  if (placementChanged) {
    m_layoutDirty = true;
    m_needsMeasure = true;
    m_measuredClientSize = {};
  }
  ApplyMouseTransparency();
}

void CCandidateWindow::SetStyle(const SrfUIStyle& style) {
  if (CandidateWindowStyleEquals(m_style, style)) return;
  CancelPendingHorizontalShrink();
  const bool topmostChanged = m_style.candidateTopmost != style.candidateTopmost;
  const bool opacityChanged = m_style.candidateOpacity != style.candidateOpacity;
  const bool abbreviateChanged =
      m_style.candidateAbbreviateLength != style.candidateAbbreviateLength;
  const bool horizontalChanged = m_style.candidateHorizontal != style.candidateHorizontal;
  const bool cornerRadiusChanged =
      m_style.skinLoaded != style.skinLoaded ||
      m_style.skinCornerRadius != style.skinCornerRadius ||
      m_style.candidateLayoutVariant != style.candidateLayoutVariant;
  const bool layoutChanged = !CandidateWindowLayoutStyleEquals(m_style, style);
  const bool fontChanged = !CandidateWindowResourceStyleEquals(m_style, style);
  const bool paintChanged = !CandidateWindowPaintStyleEquals(m_style, style);
  m_style = style;
  m_pendingStyleUpdate = true;
  m_pendingLayoutStyleUpdate = layoutChanged;
  if (!m_style.candidateRightClick && m_pinMenuVisible) {
    m_pinMenuVisible = false;
    m_pinMenuIndex = 0;
    m_pinMenuItemPinned = false;
    m_pinMenuHotCommand = kPinMenuCommandNone;
    m_pinMenuPressedCommand = kPinMenuCommandNone;
    m_pinMenuRect = {};
    m_pinMenuPinRect = {};
    m_pinMenuUnpinRect = {};
    m_pinMenuRemoveRect = {};
    m_pinMenuBlockRect = {};
    m_pinMenuSourceRect = {};
    m_layoutDirty = true;
    m_staticPaintDirty = true;
  }
  if (abbreviateChanged || horizontalChanged) RebuildDisplayItems();
  if (layoutChanged) {
    m_layoutDirty = true;
    m_needsMeasure = true;
    m_staticPaintDirty = true;
    m_measuredClientSize = {};
    m_itemRects.clear();
    m_pinMenuRect = {};
    m_pinMenuPinRect = {};
    m_pinMenuUnpinRect = {};
    m_pinMenuRemoveRect = {};
    m_pinMenuBlockRect = {};
    m_pinMenuSourceRect = {};
    ReleaseItemPaintCaches();
  } else if (paintChanged || fontChanged) {
    m_fullPaintDirty = true;
    m_staticPaintDirty = true;
    ReleaseItemPaintCaches();
  }
  if (fontChanged) m_fontsDirty = true;
  if (!MotionEnabled() &&
      (m_showAnimationActive || m_selectionAnimationActive || m_hoverAnimationActive ||
       m_pressAnimationActive || m_pageAnimationActive)) {
    CancelAnimations(true);
  }
  if (opacityChanged) {
    ApplyWindowOpacity();
    UpdateShadowWindow();
  }
  if (topmostChanged && m_hwnd) {
    SetWindowPos(m_hwnd, m_style.candidateTopmost ? HWND_TOPMOST : HWND_NOTOPMOST, 0, 0, 0, 0,
                 SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
    if (m_shadowHwnd) {
      SetWindowPos(m_shadowHwnd, m_hwnd, 0, 0, 0, 0,
                   SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
    }
  }
  if (cornerRadiusChanged && m_hwnd) {
    RECT client = {};
    GetClientRect(m_hwnd, &client);
    ApplyWindowRegion(client.right - client.left, client.bottom - client.top, TRUE);
    UpdateShadowWindow();
  }
  if (m_hwnd) InvalidateRect(m_hwnd, nullptr, FALSE);
}

void CCandidateWindow::PrepareResources() {
  if (!m_fontsDirty && m_resourcesPrepared && m_renderResourcesPrimed) return;
  RefreshFonts();
  if (!m_renderResourcesPrimed) {
    m_renderResourcesPrimed = PrimeRenderResources();
  }
}

bool CCandidateWindow::PrimeRenderResources() {
  return PrimeCandidateRenderResources(m_titleFont, m_bodyFont, m_bodyStrongFont, m_metaFont,
                                       m_labelFont, m_chipFont);
}

void CCandidateWindow::Show(const std::wstring& title, const std::vector<std::wstring>& pageItems,
                            const std::vector<std::wstring>& pageComments,
                            const std::vector<std::wstring>& pageLabels,
                            const std::vector<bool>& pagePinnedItems,
                            const std::vector<bool>& pageClipboardItems, UINT pageIndex,
                            UINT totalPages, UINT selectedInPage, const RECT& anchorRect,
                            const std::vector<std::wstring>& modeTags, bool interactive,
                            bool pendingVisual) {
  if (!EnsureWindow()) return;
  const bool wasVisible = IsWindowVisible(m_hwnd) != FALSE;
  const bool interactionChanged =
      m_interactive != interactive || m_pendingVisual != pendingVisual;
  if (interactionChanged && (!interactive || pendingVisual)) {
    if (GetCapture() == m_hwnd) ReleaseCapture();
    m_hotIndex = -1;
    m_pressedIndex = -1;
    m_rightPressedIndex = -1;
    m_pinMenuVisible = false;
    m_pinMenuIndex = 0;
    m_pinMenuItemPinned = false;
    m_pinMenuHotCommand = kPinMenuCommandNone;
    m_pinMenuPressedCommand = kPinMenuCommandNone;
    m_pinMenuRect = {};
    m_pinMenuPinRect = {};
    m_pinMenuUnpinRect = {};
    m_pinMenuRemoveRect = {};
    m_pinMenuBlockRect = {};
    m_pinMenuSourceRect = {};
  }

  std::vector<std::wstring> nextComments(pageItems.size(), std::wstring());
  std::vector<std::wstring> nextLabels(pageItems.size(), std::wstring());
  std::vector<bool> nextPinned(pageItems.size(), false);
  std::vector<bool> nextClipboard(pageItems.size(), false);
  for (size_t i = 0; i < pageItems.size(); ++i) {
    if (i < pageComments.size()) nextComments[i] = pageComments[i];
    if (i < pageLabels.size()) nextLabels[i] = pageLabels[i];
    if (nextLabels[i].empty()) nextLabels[i] = std::to_wstring(i + 1);
    if (i < pagePinnedItems.size()) nextPinned[i] = pagePinnedItems[i];
    if (i < pageClipboardItems.size()) nextClipboard[i] = pageClipboardItems[i];
  }
  const UINT nextTotalPages = std::max(1u, totalPages);
  // The window keeps a zero-based page index. Callers that expose a page
  // number to users convert to 1-based only when formatting the badge.
  const UINT nextPageIndex = std::min(pageIndex, nextTotalPages - 1);
  const UINT nextSelectedInPage =
      pageItems.empty() ? 0 : std::min(selectedInPage, static_cast<UINT>(pageItems.size() - 1));
  const bool selectionChangesClipboardLayout =
      nextSelectedInPage != m_selectedInPage && HasClipboardCandidateItems(nextClipboard);
  const UINT nextDpi = DpiForScreenRect(&anchorRect);
  if (m_pinMenuVisible &&
      (m_title != title || m_items != pageItems || m_comments != nextComments ||
       m_labels != nextLabels || m_pinnedItems != nextPinned ||
       m_clipboardItems != nextClipboard ||
       m_modeTags != modeTags || m_pageIndex != nextPageIndex ||
       m_totalPages != nextTotalPages || m_pinMenuIndex >= pageItems.size() ||
       m_pinMenuIndex != nextSelectedInPage)) {
    m_pinMenuVisible = false;
    m_pinMenuIndex = 0;
    m_pinMenuItemPinned = false;
    m_pinMenuHotCommand = kPinMenuCommandNone;
    m_pinMenuPressedCommand = kPinMenuCommandNone;
    m_pinMenuRect = {};
    m_pinMenuPinRect = {};
    m_pinMenuUnpinRect = {};
    m_pinMenuRemoveRect = {};
    m_pinMenuBlockRect = {};
    m_pinMenuSourceRect = {};
    m_layoutDirty = true;
  }

  if (!selectionChangesClipboardLayout && !m_layoutDirty && m_hasAnchorRect &&
      m_title == title && m_items == pageItems &&
      m_comments == nextComments &&
      m_labels == nextLabels && m_pinnedItems == nextPinned &&
      m_clipboardItems == nextClipboard &&
      m_modeTags == modeTags && m_pageIndex == nextPageIndex && m_totalPages == nextTotalPages &&
      m_interactive == interactive && m_pendingVisual == pendingVisual &&
      EqualRect(&m_anchorRect, &anchorRect) && m_dpi == nextDpi &&
      m_lastLayoutHorizontal == m_style.candidateHorizontal &&
      m_lastLayoutVariant == m_style.candidateLayoutVariant) {
    const UINT previousSelected = m_selectedInPage;
    const bool needsFullPaint = m_fullPaintDirty;
    m_selectedInPage = nextSelectedInPage;
    const CandidateWindowUpdateKind updateKind =
        previousSelected != m_selectedInPage
            ? CandidateWindowUpdateKind::Selection
            : (m_pendingStyleUpdate
                   ? (m_pendingLayoutStyleUpdate ? CandidateWindowUpdateKind::Layout
                                                 : CandidateWindowUpdateKind::Style)
                   : CandidateWindowUpdateKind::Content);
    if (updateKind == CandidateWindowUpdateKind::Content ||
        updateKind == CandidateWindowUpdateKind::Layout ||
        updateKind == CandidateWindowUpdateKind::Style) {
      m_selectionAnimationActive = false;
      m_pageAnimationActive = false;
    }
    const ULONGLONG showTick = GetTickCount64();
    const bool deferImmediatePaint =
        ShouldDeferImmediatePaint(updateKind, wasVisible, m_lastShowTick, showTick);
    if (!deferImmediatePaint) m_lastShowTick = showTick;
    if (!IsWindowVisible(m_hwnd)) {
      StartShowAnimation();
      ShowWindow(m_hwnd, SW_SHOWNOACTIVATE);
      ShowShadowWindow();
    } else {
      ShowShadowWindow();
    }
    ScheduleOverlayEnvironmentPoll();
    if (previousSelected != m_selectedInPage) {
      StartSelectionAnimation(static_cast<int>(previousSelected),
                              static_cast<int>(m_selectedInPage));
      if (!needsFullPaint) {
        InvalidateCandidateIndex(static_cast<int>(previousSelected));
        InvalidateCandidateIndex(static_cast<int>(m_selectedInPage));
      }
    }
    if (needsFullPaint) {
      InvalidateRect(m_hwnd, nullptr, FALSE);
    }
    if ((previousSelected != m_selectedInPage || needsFullPaint) && !deferImmediatePaint) {
      FlushPendingInvalidates();
      UpdateWindow(m_hwnd);
    } else if ((previousSelected != m_selectedInPage || needsFullPaint) && deferImmediatePaint) {
      ScheduleDeferredPaint(showTick);
    }
    m_pendingStyleUpdate = false;
    m_pendingLayoutStyleUpdate = false;
    m_lastLayoutHorizontal = m_style.candidateHorizontal;
    m_lastLayoutVariant = m_style.candidateLayoutVariant;
    return;
  }

  const bool previousHadAnchor = m_hasAnchorRect;
  const bool sameInteractiveContext =
      m_hasAnchorRect && m_title == title && m_modeTags == modeTags &&
      m_totalPages == nextTotalPages && EqualRect(&m_anchorRect, &anchorRect) &&
      m_dpi == nextDpi && m_lastLayoutHorizontal == m_style.candidateHorizontal &&
      m_lastLayoutVariant == m_style.candidateLayoutVariant;
  CandidateWindowUpdateKind updateKind = CandidateWindowUpdateKind::Content;
  if (m_pendingStyleUpdate) {
    updateKind =
        m_pendingLayoutStyleUpdate ? CandidateWindowUpdateKind::Layout : CandidateWindowUpdateKind::Style;
  } else if (m_layoutDirty || !previousHadAnchor || !EqualRect(&m_anchorRect, &anchorRect) ||
             m_dpi != nextDpi || m_lastLayoutHorizontal != m_style.candidateHorizontal ||
             m_lastLayoutVariant != m_style.candidateLayoutVariant) {
    updateKind = CandidateWindowUpdateKind::Layout;
  } else if (sameInteractiveContext && m_pageIndex != nextPageIndex) {
    updateKind = CandidateWindowUpdateKind::Page;
  }
  if (updateKind == CandidateWindowUpdateKind::Content ||
      updateKind == CandidateWindowUpdateKind::Layout ||
      updateKind == CandidateWindowUpdateKind::Style) {
    m_selectionAnimationActive = false;
    m_pageAnimationActive = false;
  }

  const ULONGLONG showTick = GetTickCount64();
  const bool deferImmediatePaint =
      ShouldDeferImmediatePaint(updateKind, wasVisible, m_lastShowTick, showTick);
  if (!deferImmediatePaint) m_lastShowTick = showTick;

  const bool itemsChanged = m_items != pageItems;
  const bool commentsChanged = m_comments != nextComments;
  const bool labelsChanged = m_labels != nextLabels;
  const bool clipboardItemsChanged = m_clipboardItems != nextClipboard;
  const bool pinnedItemsChanged = m_pinnedItems != nextPinned;
  const bool abbreviateChanged =
      m_lastDisplayAbbreviateLength != m_style.candidateAbbreviateLength;
  const SIZE previousMeasuredClientSize = m_measuredClientSize;
  const std::vector<RECT> previousItemRects = m_itemRects;
  const UINT previousSelectedInPage = m_selectedInPage;
  const UINT previousPageIndex = m_pageIndex;
  const bool staticLayerUnchanged = m_style.candidateHorizontal
                                        ? (m_pageIndex == nextPageIndex &&
                                           m_totalPages == nextTotalPages &&
                                           m_pendingVisual == pendingVisual)
                                        : (m_title == title && m_modeTags == modeTags &&
                                           m_pageIndex == nextPageIndex &&
                                           m_totalPages == nextTotalPages);
  std::vector<size_t> changedItemIndices;
  if (m_items.size() == pageItems.size() && m_comments.size() == nextComments.size() &&
      m_labels.size() == nextLabels.size() && m_clipboardItems.size() == nextClipboard.size()) {
    for (size_t i = 0; i < pageItems.size(); ++i) {
      if (m_items[i] != pageItems[i] || m_comments[i] != nextComments[i] ||
          m_labels[i] != nextLabels[i] || m_clipboardItems[i] != nextClipboard[i]) {
        changedItemIndices.push_back(i);
      }
    }
  }
  const bool itemPaintCacheFullyInvalid =
      m_layoutDirty || m_items.size() != pageItems.size() || abbreviateChanged ||
      clipboardItemsChanged || pinnedItemsChanged || interactionChanged ||
      m_dpi != nextDpi || m_lastLayoutHorizontal != m_style.candidateHorizontal ||
      m_lastLayoutVariant != m_style.candidateLayoutVariant;
  if (itemPaintCacheFullyInvalid) {
    ReleaseItemPaintCaches();
  } else if (itemsChanged || commentsChanged || labelsChanged || clipboardItemsChanged) {
    for (size_t index : changedItemIndices) {
      ReleaseItemPaintCacheAt(index);
    }
  }
  m_title = title;
  m_items = pageItems;
  m_comments = std::move(nextComments);
  m_labels = std::move(nextLabels);
  m_pinnedItems = std::move(nextPinned);
  m_clipboardItems = std::move(nextClipboard);
  UpdatePendingIndicatorTimer(pendingVisual);
  m_interactive = interactive;
  m_pendingVisual = pendingVisual;
  m_modeTags = modeTags;
  m_selectedInPage = nextSelectedInPage;
  if (itemsChanged || commentsChanged || abbreviateChanged || clipboardItemsChanged ||
      selectionChangesClipboardLayout) {
    RebuildDisplayItems();
  }
  m_pageIndex = nextPageIndex;
  m_totalPages = nextTotalPages;
  m_anchorRect = anchorRect;
  m_hasAnchorRect = true;
  const UINT previousDpi = m_dpi;
  m_dpi = nextDpi;
  if (!m_titleFont || !m_bodyFont || !m_bodyStrongFont || !m_metaFont || !m_labelFont ||
      !m_chipFont || previousDpi != nextDpi) {
    m_fontsDirty = true;
  }
  if (m_fontsDirty) {
    RefreshFonts();
  }

  const RECT work = PlacementAreaForAnchor(&anchorRect, m_fullscreenOverlayPlacement);
  const int screenInset = std::max(Scale(10), CandidateShadowMarginForStyle(m_style, m_dpi));
  const int maxWidth =
      std::max(Scale(260), static_cast<int>((work.right - work.left) - screenInset * 2));
  m_measuredClientSize = MeasureClientSize(maxWidth, &m_itemRects);
  const SIZE naturalMeasuredClientSize = m_measuredClientSize;
  const std::vector<RECT> naturalItemRects = m_itemRects;
  // Keep visible content updates from shrinking the popup while lookup results are still
  // churning. 3-4 character readings can produce several complete candidate passes with
  // different widths; resizing on every pass reads as flicker.
  // Keep the old geometry briefly so an asynchronous pass cannot move a
  // clickable item, then apply the latest natural width once results settle.
  const bool delayHorizontalShrink = ShouldDelayHorizontalCandidateShrink(
      m_style.candidateHorizontal, wasVisible, previousHadAnchor,
      previousMeasuredClientSize.cx, naturalMeasuredClientSize.cx,
      updateKind == CandidateWindowUpdateKind::Content);
  const bool keepPreviousWidth =
      wasVisible && previousHadAnchor && previousMeasuredClientSize.cx > 0 &&
      (delayHorizontalShrink ||
       (!m_style.candidateHorizontal && updateKind != CandidateWindowUpdateKind::Style));
  if (keepPreviousWidth) {
    m_measuredClientSize.cx = std::min<LONG>(
        static_cast<LONG>(maxWidth),
        std::max<LONG>(m_measuredClientSize.cx, previousMeasuredClientSize.cx));
  }
  if (delayHorizontalShrink) {
    SchedulePendingHorizontalShrink(anchorRect, naturalMeasuredClientSize, naturalItemRects);
  } else {
    CancelPendingHorizontalShrink();
  }
  m_needsMeasure = false;
  m_lastLayoutHorizontal = m_style.candidateHorizontal;
  m_lastLayoutVariant = m_style.candidateLayoutVariant;
  const RECT rect = CalculateWindowRect(anchorRect, m_measuredClientSize);
  ApplyWindowRect(rect);
  const bool geometryUnchanged =
      wasVisible && previousHadAnchor && updateKind == CandidateWindowUpdateKind::Content &&
      !m_fullPaintDirty && !m_staticPaintDirty &&
      previousMeasuredClientSize.cx == m_measuredClientSize.cx &&
      previousMeasuredClientSize.cy == m_measuredClientSize.cy &&
      RectVectorEquals(previousItemRects, m_itemRects);
  const bool canPartialRepaintContent = geometryUnchanged && staticLayerUnchanged &&
                                        !itemPaintCacheFullyInvalid &&
                                        (itemsChanged || commentsChanged || labelsChanged ||
                                         clipboardItemsChanged ||
                                         pinnedItemsChanged || interactionChanged ||
                                         previousSelectedInPage != m_selectedInPage);
  m_layoutDirty = !canPartialRepaintContent;
  if (!canPartialRepaintContent) {
    m_staticPaintDirty = true;
  }
  if (canPartialRepaintContent) {
    for (size_t index : changedItemIndices) {
      InvalidateCandidateIndex(static_cast<int>(index));
    }
    if (previousSelectedInPage != m_selectedInPage) {
      InvalidateCandidateIndex(static_cast<int>(previousSelectedInPage));
      InvalidateCandidateIndex(static_cast<int>(m_selectedInPage));
    }
  } else {
    InvalidateRect(m_hwnd, nullptr, FALSE);
  }
  if (!wasVisible) {
    StartShowAnimation();
    ShowWindow(m_hwnd, SW_SHOWNOACTIVATE);
    ShowShadowWindow();
  } else {
    ShowWindow(m_hwnd, SW_SHOWNOACTIVATE);
    ShowShadowWindow();
    if (updateKind == CandidateWindowUpdateKind::Page) {
      StartPageAnimation(static_cast<int>(previousPageIndex), static_cast<int>(m_pageIndex));
    } else if (previousSelectedInPage != m_selectedInPage &&
               RectVectorEquals(previousItemRects, m_itemRects)) {
      StartSelectionAnimation(static_cast<int>(previousSelectedInPage),
                              static_cast<int>(m_selectedInPage));
    }
  }
  ScheduleOverlayEnvironmentPoll();
  if (!deferImmediatePaint) {
    FlushPendingInvalidates();
    UpdateWindow(m_hwnd);
  } else {
    ScheduleDeferredPaint(showTick);
  }
  m_pendingStyleUpdate = false;
  m_pendingLayoutStyleUpdate = false;
}

void CCandidateWindow::Hide() {
  CancelEnvironmentRefresh();
  CancelAnimations(false);
  UpdatePendingIndicatorTimer(false);
  if (m_hwnd && m_paintTimerPending) {
    KillTimer(m_hwnd, kCandidatePaintTimerId);
    m_paintTimerPending = false;
  }
  CancelPendingHorizontalShrink();
  if (m_hwnd && GetCapture() == m_hwnd) ReleaseCapture();
  m_hotIndex = -1;
  m_pressedIndex = -1;
  m_rightPressedIndex = -1;
  m_trackingMouse = false;
  m_pinMenuVisible = false;
  m_pinMenuIndex = 0;
  m_pinMenuItemPinned = false;
  m_pinMenuHotCommand = kPinMenuCommandNone;
  m_pinMenuPressedCommand = kPinMenuCommandNone;
  m_pinMenuRect = {};
  m_pinMenuPinRect = {};
  m_pinMenuUnpinRect = {};
  m_pinMenuRemoveRect = {};
  m_pinMenuBlockRect = {};
  m_pinMenuSourceRect = {};
  // Start the next composition from its natural dimensions.
  m_measuredClientSize = {};
  m_itemRects.clear();
  m_needsMeasure = true;
  if (m_hwnd) ShowWindow(m_hwnd, SW_HIDE);
  HideShadowWindow();
}

bool CCandidateWindow::IsVisible() const {
  return m_hwnd && IsWindowVisible(m_hwnd) != FALSE;
}

bool CCandidateWindow::HasPendingPaint() const {
  return m_hwnd && (m_paintTimerPending || m_animationTimerPending || m_hasPendingDirtyRgn ||
                    m_layoutDirty || m_fullPaintDirty || m_staticPaintDirty);
}

void CCandidateWindow::FlushPendingPaint() {
  if (!m_hwnd) return;
  if (m_paintTimerPending) {
    KillTimer(m_hwnd, kCandidatePaintTimerId);
    m_paintTimerPending = false;
  }
  FlushPendingInvalidates();
  if (m_layoutDirty || m_fullPaintDirty || m_staticPaintDirty) {
    InvalidateRect(m_hwnd, nullptr, FALSE);
  }
  UpdateWindow(m_hwnd);
  m_lastShowTick = GetTickCount64();
}

void CCandidateWindow::SetPresentationState(bool interactive, bool pendingVisual) {
  if (m_interactive == interactive && m_pendingVisual == pendingVisual) return;
  if ((!interactive || pendingVisual) && m_hwnd) {
    if (GetCapture() == m_hwnd) ReleaseCapture();
    m_hotIndex = -1;
    m_pressedIndex = -1;
    m_rightPressedIndex = -1;
    m_pinMenuVisible = false;
    m_pinMenuIndex = 0;
    m_pinMenuItemPinned = false;
    m_pinMenuHotCommand = kPinMenuCommandNone;
    m_pinMenuPressedCommand = kPinMenuCommandNone;
    m_pinMenuRect = {};
    m_pinMenuPinRect = {};
    m_pinMenuUnpinRect = {};
    m_pinMenuRemoveRect = {};
    m_pinMenuBlockRect = {};
    m_pinMenuSourceRect = {};
  }
  UpdatePendingIndicatorTimer(pendingVisual);
  m_interactive = interactive;
  m_pendingVisual = pendingVisual;
  ReleaseItemPaintCaches();
  m_staticPaintDirty = true;
  m_fullPaintDirty = true;
  if (!m_hwnd || !IsWindowVisible(m_hwnd)) return;
  InvalidateRect(m_hwnd, nullptr, FALSE);
  ScheduleDeferredPaint(GetTickCount64());
}

void CCandidateWindow::Destroy() {
  Hide();
  ReleasePaintBuffer();
  ReleaseStaticPaintBuffer();
  ReleaseItemPaintCaches();
  DestroyFonts();
  if (m_hwnd) {
    DestroyWindow(m_hwnd);
    m_hwnd = nullptr;
  }
  if (m_shadowHwnd) {
    DestroyWindow(m_shadowHwnd);
    m_shadowHwnd = nullptr;
  }
}

bool CCandidateWindow::EnsureWindow() {
  if (m_hwnd) return true;
  if (!EnsureWindowClass()) return false;
  const DWORD exStyle = WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED |
                        (m_gameOverlay ? WS_EX_TRANSPARENT : 0) |
                        (m_style.candidateTopmost ? WS_EX_TOPMOST : 0);
  m_hwnd = CreateWindowExW(exStyle,
                           kCandidateWndClass, L"", WS_POPUP, CW_USEDEFAULT, CW_USEDEFAULT, 0, 0,
                           nullptr, nullptr, DllOrFallbackInstance(), this);
  ApplyWindowOpacity();
  return m_hwnd != nullptr;
}

bool CCandidateWindow::EnsureShadowWindow() {
  if (m_shadowHwnd) return true;
  const DWORD exStyle = WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT |
                        (m_style.candidateTopmost ? WS_EX_TOPMOST : 0);
  m_shadowHwnd = CreateWindowExW(exStyle, L"STATIC", L"", WS_POPUP, 0, 0, 0, 0, nullptr, nullptr,
                                 DllOrFallbackInstance(), nullptr);
  return m_shadowHwnd != nullptr;
}

void CCandidateWindow::UpdateShadowWindow() {
  if (!m_hwnd) return;
  const CandidateColors colors = ResolveColors(m_style);
  HIGHCONTRASTW highContrast = {};
  highContrast.cbSize = sizeof(highContrast);
  const bool systemHighContrast =
      SystemParametersInfoW(SPI_GETHIGHCONTRAST, sizeof(highContrast), &highContrast, 0) &&
      (highContrast.dwFlags & HCF_HIGHCONTRASTON) != 0;
  if (!colors.shadowEnabled || colors.shadowSize <= 0 || colors.shadowOpacity <= 0.001f ||
      m_style.themeMode == SrfThemeMode::HighContrast || systemHighContrast) {
    HideShadowWindow();
    return;
  }
  if (!EnsureShadowWindow()) return;

  RECT windowRect = {};
  if (!GetWindowRect(m_hwnd, &windowRect)) return;
  const int width = windowRect.right - windowRect.left;
  const int height = windowRect.bottom - windowRect.top;
  if (width <= 0 || height <= 0) return;

  const LayoutSpec spec = ResolveLayoutSpec(m_style);
  const int radius = SnapGdiRadiusForDpi(
      Scale(m_style.skinLoaded && m_style.skinCornerRadius >= 0 ? m_style.skinCornerRadius
                                                                : spec.cornerRadius),
      m_dpi);
  const int spread = std::max(1, Scale(colors.shadowSize));
  const int maxAlpha =
      std::clamp(static_cast<int>(std::lround(colors.shadowOpacity * 255.0f * 2.6f)), 1, 96);
  SoftShadowCache& cache = GetSoftShadowCache();
  if (!cache.Ensure(width, height, std::max(1, radius), spread, maxAlpha)) {
    HideShadowWindow();
    return;
  }

  float showProgress = 1.0f;
  if (m_showAnimationActive) {
    showProgress = EaseOutCubic(
        LinearAnimationProgress(GetTickCount64(), m_showAnimationStart, m_showAnimationDurationMs));
  }
  const UINT opacity = std::clamp(m_style.candidateOpacity, 70u, 100u);
  const BYTE sourceAlpha = static_cast<BYTE>(
      std::clamp(static_cast<int>(std::lround(255.0f * showProgress * opacity / 100.0f)), 0, 255));
  POINT destination = {windowRect.left - spread, windowRect.top - spread};
  SIZE size = {cache.bitmapW, cache.bitmapH};
  POINT source = {};
  BLENDFUNCTION blend = {};
  blend.BlendOp = AC_SRC_OVER;
  blend.SourceConstantAlpha = sourceAlpha;
  blend.AlphaFormat = AC_SRC_ALPHA;
  HDC screenDc = GetDC(nullptr);
  const BOOL updated = UpdateLayeredWindow(m_shadowHwnd, screenDc, &destination, &size, cache.memDc,
                                           &source, 0, &blend, ULW_ALPHA);
  if (screenDc) ReleaseDC(nullptr, screenDc);
  if (!updated) {
    HideShadowWindow();
    return;
  }
  SetWindowPos(m_shadowHwnd, m_hwnd, 0, 0, 0, 0,
               SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER);
  if (IsWindowVisible(m_hwnd)) ShowWindow(m_shadowHwnd, SW_SHOWNOACTIVATE);
}

void CCandidateWindow::ShowShadowWindow() {
  UpdateShadowWindow();
}

void CCandidateWindow::HideShadowWindow() {
  if (m_shadowHwnd) ShowWindow(m_shadowHwnd, SW_HIDE);
}

void CCandidateWindow::ApplyMouseTransparency() {
  if (!m_hwnd) return;
  const LONG_PTR current = GetWindowLongPtrW(m_hwnd, GWL_EXSTYLE);
  const LONG_PTR desired = m_gameOverlay ? (current | WS_EX_TRANSPARENT)
                                         : (current & ~static_cast<LONG_PTR>(WS_EX_TRANSPARENT));
  if (desired == current) return;
  SetWindowLongPtrW(m_hwnd, GWL_EXSTYLE, desired);
  SetWindowPos(m_hwnd, nullptr, 0, 0, 0, 0,
               SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER |
                   SWP_NOOWNERZORDER | SWP_FRAMECHANGED);
}

void CCandidateWindow::ApplyWindowOpacity() {
  if (!m_hwnd) return;
  const UINT opacity = std::clamp(m_style.candidateOpacity, 70u, 100u);
  float progress = 1.0f;
  if (m_showAnimationActive) {
    progress = EaseOutCubic(
        LinearAnimationProgress(GetTickCount64(), m_showAnimationStart, m_showAnimationDurationMs));
  }
  const BYTE alpha = static_cast<BYTE>(std::clamp(
      static_cast<int>(std::lround(MulDiv(static_cast<int>(opacity), 255, 100) * progress)), 0,
      255));
  SetLayeredWindowAttributes(m_hwnd, 0, alpha, LWA_ALPHA);
}

void CCandidateWindow::RefreshFonts() {
  if (m_dpi == 0) m_dpi = CurrentDpi(m_hwnd);

  const UINT bodyPt = std::clamp(m_style.candidateFontSize, 10u, 28u);
  const UINT titlePt = std::min(bodyPt + 1, 30u);
  const UINT metaPt = bodyPt > 11 ? bodyPt - 2 : 9u;
  const UINT labelPt = bodyPt > 10 ? bodyPt - 1 : bodyPt;
  const UINT chipPt = bodyPt > 11 ? bodyPt - 3 : 8u;
  const int bodyWeight = ResolveFontWeight(m_style.candidateFontWeight, 500, 300, 700);
  const int strongWeight =
      ResolveFontWeight(m_style.candidateSelectedFontWeight, 600, 400, 800);
  const int labelWeight = ResolveFontWeight(m_style.candidateLabelFontWeight, 600, 400, 800);
  const int chipWeight = ResolveFontWeight(m_style.candidateChipFontWeight, 500, 350, 700);

  const std::filesystem::path customFontPath =
      !m_style.candidateFontFile.empty() ? ResolveCandidateFontPath(m_style.candidateFontFile)
                                         : std::filesystem::path();
  const std::wstring effectiveFontFile =
      !m_style.candidateFontFile.empty() ? m_style.candidateFontFile : kDefaultCandidateFontFace;
  const PreparedFontEntry prepared = EnsurePreparedFontEntry(customFontPath);
  const std::wstring customFontFace =
      ResolveConfiguredFontFaceName(m_style.candidateFontFile, customFontPath, prepared);

  // 如果字体 key 没有变化，直接复用现有字体句柄，避免重复创建。
  if (!m_fontsDirty && m_resourcesPrepared &&
      m_lastFontDpi == m_dpi && m_lastFontFace == customFontFace &&
      m_lastFontSize == bodyPt && m_lastFontWeight == bodyWeight &&
      m_lastSelectedWeight == strongWeight && m_lastLabelWeight == labelWeight &&
      m_lastChipWeight == chipWeight && m_lastFontFile == effectiveFontFile) {
    return;
  }

  DestroyFonts();
  m_customFontPath = customFontPath;
  m_customFontFace = customFontFace;

  const UINT fontDpi = static_cast<UINT>(std::clamp(
      MulDiv(static_cast<int>(m_dpi),
             static_cast<int>(std::clamp(m_style.candidateScalePercent, 50u, 200u)), 100),
      48, 1536));
  auto makeFont = [&](UINT points, LONG weight) -> HFONT {
    return CreateFontW(PointSizeToPixels(points, fontDpi), 0, 0, 0, weight, FALSE, FALSE, FALSE,
                       DEFAULT_CHARSET, OUT_TT_PRECIS, CLIP_DEFAULT_PRECIS,
                       CLEARTYPE_NATURAL_QUALITY, DEFAULT_PITCH | FF_SWISS,
                       m_customFontFace.c_str());
  };

  m_titleFont = makeFont(titlePt, FW_SEMIBOLD);
  m_bodyFont = makeFont(bodyPt, bodyWeight);
  // 选中项保留“略强调”，避免过粗导致笔画糊成一团。
  m_bodyStrongFont = makeFont(bodyPt, strongWeight);
  m_metaFont = makeFont(metaPt, FW_NORMAL);
  m_labelFont = makeFont(labelPt, labelWeight);
  m_chipFont = makeFont(chipPt, chipWeight);

  // 如果自定义字体缺字/加载异常（大量方框），兜底回退到微软雅黑。
  // 说明：选择自定义字体时我们优先完整显示候选；字体缺字会严重影响可读性，因此宁可回退。
  if (m_customFontFace != kDefaultCandidateFontFace) {
    HDC dc = GetDC(m_hwnd ? m_hwnd : nullptr);
    const bool ok = dc && FontSupportsProbeText(dc, m_bodyStrongFont);
    if (dc) ReleaseDC(m_hwnd ? m_hwnd : nullptr, dc);
    if (!ok) {
      DestroyFonts();
      m_customFontFace = kDefaultCandidateFontFace;
      m_titleFont = makeFont(titlePt, FW_SEMIBOLD);
      m_bodyFont = makeFont(bodyPt, bodyWeight);
      m_bodyStrongFont = makeFont(bodyPt, strongWeight);
      m_metaFont = makeFont(metaPt, FW_NORMAL);
      m_labelFont = makeFont(labelPt, labelWeight);
      m_chipFont = makeFont(chipPt, chipWeight);
    }
  }
  GetDirectTextRenderer().ConfigureCustomFont(m_customFontPath, m_customFontFace);
  m_lastFontDpi = m_dpi;
  m_lastFontFace = m_customFontFace;
  m_lastFontSize = bodyPt;
  m_lastFontWeight = bodyWeight;
  m_lastSelectedWeight = strongWeight;
  m_lastLabelWeight = labelWeight;
  m_lastChipWeight = chipWeight;
  m_lastFontFile = effectiveFontFile;
  m_fontsDirty = false;
  m_resourcesPrepared = true;
}

void CCandidateWindow::DestroyFonts() {
  DeleteGdiObject(m_titleFont);
  DeleteGdiObject(m_bodyFont);
  DeleteGdiObject(m_bodyStrongFont);
  DeleteGdiObject(m_metaFont);
  DeleteGdiObject(m_labelFont);
  DeleteGdiObject(m_chipFont);
  m_titleFont = nullptr;
  m_bodyFont = nullptr;
  m_bodyStrongFont = nullptr;
  m_metaFont = nullptr;
  m_labelFont = nullptr;
  m_chipFont = nullptr;
  m_customFontPath.clear();
  m_customFontFace.clear();
  GetDirectTextRenderer().ConfigureCustomFont({}, {});
  m_measuredClientSize = {};
  m_staticPaintDirty = true;
  m_fontsDirty = true;
  m_resourcesPrepared = false;
  m_renderResourcesPrimed = false;
  ClearMeasuredTextCaches();
  ReleaseItemPaintCaches();
}

void CCandidateWindow::RebuildDisplayItems() {
  m_displayItems.clear();
  m_displayItems.reserve(m_items.size());
  // Split clipboard comments once per candidate update instead of per frame
  // in the measure and draw loops.  Mirrors the draw-site condition
  // (clipboardMode && i < m_comments.size()).
  const bool clipboardMode =
      !m_style.candidateHorizontal && HasClipboardCandidateItems(m_clipboardItems);
  m_clipboardCommentParts.clear();
  m_clipboardCommentParts.reserve(m_items.size());
  for (size_t i = 0; i < m_items.size(); ++i) {
    const bool clipboardItem =
        i < m_clipboardItems.size() && m_clipboardItems[i];
    m_clipboardCommentParts.push_back(
        (clipboardMode && i < m_comments.size())
            ? SplitClipboardComment(m_comments[i])
            : ClipboardCommentParts{});
    if (clipboardItem && !m_style.candidateHorizontal) {
      const size_t maxLines = i == static_cast<size_t>(m_selectedInPage) ? 3 : 2;
      m_displayItems.push_back(ClipboardCandidatePreviewForDisplay(m_items[i], maxLines));
      continue;
    }

    m_displayItems.push_back(
        AbbreviateCandidateForDisplay(m_items[i], m_style.candidateAbbreviateLength));
  }
  m_lastDisplayAbbreviateLength = m_style.candidateAbbreviateLength;
}

int CCandidateWindow::Scale(int value) const {
  const int dpi = static_cast<int>(m_dpi == 0 ? 96u : m_dpi);
  const int scale = static_cast<int>(std::clamp(m_style.candidateScalePercent, 50u, 200u));
  return MulDiv(value, dpi * scale, 9600);
}

SIZE CCandidateWindow::MeasureClientSize(int maxWidth, std::vector<RECT>* outRects) {
  const LayoutSpec spec = ResolveLayoutSpec(m_style);
  const int outerPadX = Scale(spec.outerPadX);
  const int outerPadY = Scale(spec.outerPadY);
  const int headerPadX = Scale(spec.headerPadX);
  const int headerPadY = Scale(spec.headerPadY);
  const int headerGap = Scale(spec.headerGap);
  const int itemGap = Scale(spec.itemGap);
  const int itemPadX = Scale(spec.itemPadX);
  const int itemPadY = Scale(spec.itemPadY);
  int labelWidth = Scale(spec.labelWidth);
  const int labelGap = Scale(spec.labelGap);
  const int commentGap = Scale(spec.commentGap);
  int minWidth = Scale(spec.minWidth);
  int preferredWidth = Scale(spec.preferredWidth);
  int maxPreferredWidth = Scale(spec.maxWidth);
  const int minHorizontalCardWidth = Scale(spec.minHorizontalCardWidth);

  HDC screenDc = GetDC(m_hwnd ? m_hwnd : nullptr);
  if (!screenDc) return {minWidth, Scale(120)};

  // Make horizontal layout more compact (measured in scaled pixels).
  const bool isHorizontal = m_style.candidateHorizontal;
  const bool clipboardMode = !isHorizontal && HasClipboardCandidateItems(m_clipboardItems);
  const bool horizontalPageIndicator = isHorizontal && m_totalPages > 1;
  const int horizontalPageIndicatorGap =
      horizontalPageIndicator ? Scale(kHorizontalPageBadgeGap) : 0;
  if (clipboardMode) {
    minWidth = std::max(minWidth, Scale(420));
    preferredWidth = std::max(preferredWidth, Scale(560));
    maxPreferredWidth = std::max(maxPreferredWidth, Scale(720));
    labelWidth = Scale(22);
  }
  const bool useHorizontalCompact = isHorizontal && (m_style.candidateHorizontalCompact || m_style.skinLoaded);
  int compact1 = 0;
  if (isHorizontal) {
    compact1 = HorizontalCompactDeltaForDpi(m_style, m_dpi);
  }
  const int compact2 = useHorizontalCompact ? std::max(Scale(2), compact1 * 2) : 0;
  const int hOuterPadX = isHorizontal ? std::max(0, outerPadX - compact1) : outerPadX;
  const int hOuterPadY = isHorizontal ? std::max(0, outerPadY - compact1) : outerPadY;
  const int hItemGap = isHorizontal ? std::max(0, itemGap - compact1) : itemGap;
  const int hItemPadX = isHorizontal ? std::max(0, itemPadX - compact1) : itemPadX;
  const int hItemPadY = isHorizontal ? std::max(0, itemPadY - compact1) : itemPadY;
  const int hBadgeHeight = isHorizontal ? std::max(Scale(16), Scale(22) - compact2) : Scale(22);
  const int hBadgeGap = isHorizontal ? std::max(0, Scale(4) - compact1) : Scale(4);

  const bool showPageBadge = m_totalPages > 1;
  const std::wstring pageText = CandidatePageIndicatorText(m_pageIndex, m_totalPages);
  const SIZE pageSize = showPageBadge ? MeasureSingleLine(screenDc, m_metaFont, pageText, m_dpi) : SIZE{};
  const SIZE titleSize = MeasureSingleLine(screenDc, m_titleFont, m_title, m_dpi);
  const int modeChipGap = Scale(4);
  const int modeChipPadX = Scale(m_style.skinLoaded ? 6 : 7);
  int modeChipWidth = 0;
  for (const auto& tag : m_modeTags) {
    if (tag.empty()) continue;
    const SIZE tagSize = MeasureSingleLine(screenDc, m_chipFont, tag, m_dpi);
    modeChipWidth += std::max(Scale(30), static_cast<int>(tagSize.cx) + modeChipPadX * 2);
    modeChipWidth += modeChipGap;
  }
  if (modeChipWidth > 0) modeChipWidth -= modeChipGap;
  const int pageBadgeWidth =
      showPageBadge ? std::max(Scale(kHorizontalPageBadgeMinWidth),
                                static_cast<int>(pageSize.cx) + Scale(12))
                    : 0;
  const int horizontalPageIndicatorWidth = horizontalPageIndicator ? pageBadgeWidth : 0;
  const int horizontalPageIndicatorHeight =
      horizontalPageIndicator
          ? std::max(hBadgeHeight, static_cast<int>(pageSize.cy) +
                                       Scale(kHorizontalPageBadgePaddingY))
          : 0;
  const int headerHeight = std::max(Scale(30), static_cast<int>(titleSize.cy) + headerPadY * 2);
  const int headerRightExtras =
      modeChipWidth + (showPageBadge ? pageBadgeWidth + Scale(16) : 0);
  const int headerMinWidth =
      isHorizontal ? 0 : titleSize.cx + headerRightExtras + headerPadX * 2 + Scale(10);
  // 横向候选需明显大于纵向 preferred/max，否则 wrapWidth 过窄会导致每张卡独占一行、右侧大片留白。
  const int layoutWidthCap = m_style.candidateHorizontal ? maxWidth : maxPreferredWidth;
  const int usableMaxWidth = std::max(minWidth, std::min(maxWidth, layoutWidthCap));
  if (m_displayItems.size() != m_items.size()) RebuildDisplayItems();
  const std::vector<std::wstring>& displayItems = m_displayItems;

  int clientWidth = std::clamp(preferredWidth, minWidth, usableMaxWidth);
  if (isHorizontal) {
    const int configuredCount = static_cast<int>(std::clamp(m_style.candidateHorizontalCount, 3u, 9u));
    const int visibleTarget =
        std::max(1, std::min(configuredCount, static_cast<int>(std::max<size_t>(1, m_items.size()))));
    const int stripGap = std::max(Scale(6), hItemGap + Scale(6));
    int desiredItemsWidth = 0;
    for (int i = 0; i < visibleTarget; ++i) {
      const std::wstring label = i < static_cast<int>(m_labels.size()) ? m_labels[static_cast<size_t>(i)]
                                                                       : std::to_wstring(i + 1);
      const int labelW =
          std::max(Scale(10), static_cast<int>(MeasureLabelCached(screenDc, label, static_cast<size_t>(i)).cx));
      const HFONT measureBodyFont =
          i == m_selectedInPage && m_bodyStrongFont ? m_bodyStrongFont : m_bodyFont;
      const int textW =
          i < static_cast<int>(displayItems.size())
              ? static_cast<int>(MeasureSingleLine(screenDc, measureBodyFont,
                                                   displayItems[static_cast<size_t>(i)], m_dpi).cx)
              : 0;
      const int commentW =
          (i == static_cast<int>(m_selectedInPage) &&
           i < static_cast<int>(m_comments.size()) &&
           !m_comments[static_cast<size_t>(i)].empty())
              ? static_cast<int>(
                    MeasureSingleLine(screenDc, m_metaFont, m_comments[static_cast<size_t>(i)], m_dpi).cx)
              : 0;
      const int rawItemW = hItemPadX * 2 + labelW + labelGap + std::max(textW, commentW) + Scale(4);
      desiredItemsWidth += std::max(rawItemW, Scale(36));
      if (i + 1 < visibleTarget) desiredItemsWidth += stripGap;
    }
    const int desiredClientWidth = hOuterPadX * 2 + desiredItemsWidth +
                                   horizontalPageIndicatorWidth + horizontalPageIndicatorGap;
    clientWidth = std::clamp(std::max(clientWidth, desiredClientWidth), minWidth, usableMaxWidth);
  }
  clientWidth = std::max(clientWidth, std::min(usableMaxWidth, headerMinWidth + (isHorizontal ? hOuterPadX : outerPadX) * 2));

  if (!m_items.empty() && !m_style.candidateHorizontal) {
    int widestItem = 0;
    int widestComment = 0;
    for (size_t i = 0; i < m_items.size(); ++i) {
      if (i < m_clipboardItems.size() && m_clipboardItems[i]) {
        size_t start = 0;
        while (start <= displayItems[i].size()) {
          const size_t end = displayItems[i].find(L'\n', start);
          const std::wstring line =
              displayItems[i].substr(start, end == std::wstring::npos ? std::wstring::npos : end - start);
          widestItem = std::max(
              widestItem,
              static_cast<int>(MeasureSingleLine(screenDc, m_bodyFont, line, m_dpi).cx));
          if (end == std::wstring::npos) break;
          start = end + 1;
        }
      } else {
        widestItem = std::max(
            widestItem,
            static_cast<int>(MeasureSingleLine(screenDc, m_bodyFont, displayItems[i], m_dpi).cx));
      }
      if (i == static_cast<size_t>(m_selectedInPage) ||
          (i < m_clipboardItems.size() && m_clipboardItems[i])) {
        widestComment = std::max(
            widestComment,
            static_cast<int>(MeasureSingleLine(screenDc, m_metaFont, m_comments[i], m_dpi).cx));
      }
    }
    const int desired = outerPadX * 2 + itemPadX * 2 + labelWidth + labelGap +
                        std::max(widestItem, widestComment) + Scale(20);
    clientWidth = std::max(clientWidth, std::min(usableMaxWidth, desired));
  }
  clientWidth = std::clamp(clientWidth, minWidth, usableMaxWidth);
  if (m_pinMenuVisible) {
    clientWidth = std::clamp(std::max(clientWidth, Scale(336)), minWidth, usableMaxWidth);
  }

  std::vector<RECT> localRects;
  auto& rects = outRects ? *outRects : localRects;
  rects.clear();

  const int areaLeft = isHorizontal ? hOuterPadX : outerPadX;
  const int horizontalTrailingReserve =
      horizontalPageIndicatorWidth + horizontalPageIndicatorGap;
  const int areaWidth = std::max(
      Scale(100), clientWidth - (isHorizontal ? hOuterPadX : outerPadX) * 2 -
                       (isHorizontal ? horizontalTrailingReserve : 0));
  int y = isHorizontal ? hOuterPadY : (outerPadY + headerHeight + headerGap);

  if (!isHorizontal) {
    // Unified badge height constant for vertical layout.
    const int badgeHeight = Scale(22);
    for (size_t i = 0; i < m_items.size(); ++i) {
      if (clipboardMode && i > 0 && i < m_pinnedItems.size() &&
          i - 1 < m_pinnedItems.size() && m_pinnedItems[i - 1] && !m_pinnedItems[i]) {
        y += Scale(14);
      }
      const int textWidth = std::max(Scale(72), areaWidth - itemPadX * 2 - labelWidth - labelGap);
      const int mainHeight = std::max(Scale(18), MeasureWrappedHeight(screenDc, m_bodyFont,
                                                                       displayItems[i], textWidth,
                                                                       m_dpi));
      int commentHeight = 0;
      if (!m_comments[i].empty() &&
          (clipboardMode || i == static_cast<size_t>(m_selectedInPage))) {
        if (clipboardMode) {
          const ClipboardCommentParts parts =
              i < m_clipboardCommentParts.size() ? m_clipboardCommentParts[i]
                                                 : ClipboardCommentParts{};
          const SIZE detailSize = MeasureSingleLine(screenDc, m_metaFont, parts.detail, m_dpi);
          const SIZE typeSize = MeasureSingleLine(screenDc, m_chipFont, parts.type, m_dpi);
          commentHeight = std::max<int>(Scale(16), std::max(detailSize.cy, typeSize.cy));
        } else {
          commentHeight =
              MeasureWrappedHeight(screenDc, m_metaFont, m_comments[i], textWidth, m_dpi);
        }
      }
      // Include badge height in min row height to prevent badge overflow.
      const int contentHeight = itemPadY * 2 + mainHeight + (commentHeight > 0 ? commentGap + commentHeight : 0);
      const int badgeMinHeight = itemPadY * 2 + std::max(badgeHeight, labelWidth);
      const int rowHeight = std::max(Scale(30), std::max(contentHeight, badgeMinHeight));
      rects.push_back({areaLeft, y, areaLeft + areaWidth, y + rowHeight});
      y += rowHeight + itemGap;
    }
  } else {
    // Horizontal layout: Sogou-like single strip, with optional reading above candidate text.
    const int configuredCount = static_cast<int>(std::clamp(m_style.candidateHorizontalCount, 3u, 9u));
    const int chosenN =
        std::min(configuredCount, static_cast<int>(m_items.size()));
    const int usedGap = std::max(Scale(6), hItemGap + Scale(6));

    if (chosenN > 0) {
      std::vector<int> itemWidths;
      itemWidths.reserve(static_cast<size_t>(chosenN));
      int totalWidth = usedGap * std::max(0, chosenN - 1);
      for (int idx = 0; idx < chosenN; ++idx) {
        const size_t i = static_cast<size_t>(idx);
        const std::wstring label = i < m_labels.size() ? m_labels[i] : std::to_wstring(idx + 1);
        const SIZE labelSize = MeasureLabelCached(screenDc, label, i);
        const HFONT measureBodyFont =
            idx == m_selectedInPage && m_bodyStrongFont ? m_bodyStrongFont : m_bodyFont;
        const SIZE itemSize =
            MeasureSingleLine(screenDc, measureBodyFont, displayItems[i], m_dpi);
        const bool hasComment = idx == static_cast<int>(m_selectedInPage) &&
                                i < m_comments.size() && !m_comments[i].empty();
        const SIZE commentSize =
            hasComment ? MeasureSingleLine(screenDc, m_metaFont, m_comments[i], m_dpi) : SIZE{};
        const int rawItemW = hItemPadX * 2 + std::max(Scale(10), static_cast<int>(labelSize.cx)) +
                             labelGap +
                             std::max(static_cast<int>(itemSize.cx), static_cast<int>(commentSize.cx)) +
                             Scale(4);
        const int itemW = std::max(rawItemW, Scale(36));
        itemWidths.push_back(itemW);
        totalWidth += itemW;
      }

      if (totalWidth > areaWidth) {
        const int availableItemsWidth = std::max(Scale(36), areaWidth - usedGap * std::max(0, chosenN - 1));
        // Keep long candidates readable. The old 36px floor made a five-item
        // strip collapse into ellipses even though the paging policy had
        // already selected a smaller page for narrow/long content.
        const int minItemWidth = std::max(Scale(56), minHorizontalCardWidth);
        int currentItemsWidth = totalWidth - usedGap * std::max(0, chosenN - 1);
        while (currentItemsWidth > availableItemsWidth) {
          auto widest = std::max_element(itemWidths.begin(), itemWidths.end());
          if (widest == itemWidths.end() || *widest <= minItemWidth) break;
          const int delta = std::min(*widest - minItemWidth, currentItemsWidth - availableItemsWidth);
          *widest -= delta;
          currentItemsWidth -= delta;
        }
      }

      int x = areaLeft;
      int maxBottom = y;
      for (int idx = 0; idx < chosenN; ++idx) {
        const size_t i = static_cast<size_t>(idx);
        const std::wstring label = i < m_labels.size() ? m_labels[i] : std::to_wstring(idx + 1);
        const SIZE labelSize = MeasureLabelCached(screenDc, label, i);
        const HFONT measureBodyFont =
            idx == m_selectedInPage && m_bodyStrongFont ? m_bodyStrongFont : m_bodyFont;
        const SIZE itemSize =
            MeasureSingleLine(screenDc, measureBodyFont, displayItems[i], m_dpi);
        const bool hasComment = idx == static_cast<int>(m_selectedInPage) &&
                                i < m_comments.size() && !m_comments[i].empty();
        const SIZE commentSize =
            hasComment ? MeasureSingleLine(screenDc, m_metaFont, m_comments[i], m_dpi) : SIZE{};
        const int mainH = std::max({Scale(18), static_cast<int>(labelSize.cy), static_cast<int>(itemSize.cy)});
        const int commentH = hasComment ? std::max(Scale(12), static_cast<int>(commentSize.cy)) : 0;
        const int stackH = mainH + (commentH > 0 ? commentGap + commentH : 0);
        const int itemW = itemWidths[static_cast<size_t>(idx)];
        const int itemH = std::max(commentH > 0 ? Scale(42) : Scale(30), hItemPadY * 2 + stackH);
        RECT rect = {x, y, x + itemW, y + itemH};
        rects.push_back(rect);
        x = rect.right + usedGap;
        maxBottom = std::max(maxBottom, static_cast<int>(rect.bottom));
      }
      y = maxBottom;
    }

    int contentRight = hOuterPadX;
    for (const RECT& r : rects) {
      contentRight = std::max(contentRight, static_cast<int>(r.right));
    }
    clientWidth = std::clamp(
        std::max({contentRight + hOuterPadX + horizontalTrailingReserve, minWidth}), minWidth,
        usableMaxWidth);
  }

  ReleaseDC(m_hwnd ? m_hwnd : nullptr, screenDc);
  const int contentBottom = rects.empty()
                                ? (isHorizontal ? hOuterPadY + Scale(30)
                                                : outerPadY + headerHeight)
                                : (isHorizontal
                                       ? std::max(static_cast<int>(rects.back().bottom),
                                                  hOuterPadY + horizontalPageIndicatorHeight)
                                       : static_cast<int>(rects.back().bottom));
  int finalBottom = contentBottom + (isHorizontal ? hOuterPadY : outerPadY);
  if (clipboardMode) {
    finalBottom += Scale(8) + Scale(28);
  }
  if (m_pinMenuVisible) {
    LayoutPinMenuRect(clientWidth, contentBottom);
    finalBottom = m_pinMenuRect.bottom + (isHorizontal ? hOuterPadY : outerPadY);
  } else {
    m_pinMenuRect = {};
    m_pinMenuPinRect = {};
    m_pinMenuUnpinRect = {};
    m_pinMenuRemoveRect = {};
    m_pinMenuBlockRect = {};
    m_pinMenuSourceRect = {};
  }
  return {clientWidth, finalBottom};
}

RECT CCandidateWindow::CalculateWindowRect(const RECT& anchorRect, SIZE content) {
  const RECT work = PlacementAreaForAnchor(&anchorRect, m_fullscreenOverlayPlacement);
  const int screenMargin = std::max(Scale(10), CandidateShadowMarginForStyle(m_style, m_dpi));
  const int gap = Scale(6);
  if (content.cx <= 0 || content.cy <= 0) {
    const int maxWidth =
        std::max(Scale(260), static_cast<int>((work.right - work.left) - screenMargin * 2));
    content = MeasureClientSize(maxWidth, nullptr);
  }

  const int minX = static_cast<int>(work.left) + screenMargin;
  const int maxX = std::max<int>(minX, static_cast<int>(work.right) - screenMargin - content.cx);
  const int minY = static_cast<int>(work.top) + screenMargin;
  const int maxY = std::max<int>(minY, static_cast<int>(work.bottom) - screenMargin - content.cy);
  int anchoredX = static_cast<int>(anchorRect.left);
  switch (m_style.candidateOverlayAnchor) {
    case SrfOverlayAnchor::TopCenter:
    case SrfOverlayAnchor::BottomCenter:
      anchoredX -= content.cx / 2;
      break;
    case SrfOverlayAnchor::TopRight:
    case SrfOverlayAnchor::BottomRight:
      anchoredX -= content.cx;
      break;
    default:
      break;
  }
  const int preferredX = std::clamp(anchoredX, minX, maxX);

  int x = preferredX;
  int y = static_cast<int>(anchorRect.bottom) + gap;
  if (y <= maxY) {
    return {x, y, x + content.cx, y + content.cy};
  }

  y = static_cast<int>(anchorRect.top) - gap - content.cy;
  if (y >= minY) {
    return {x, y, x + content.cx, y + content.cy};
  }

  if (!SidePlaceCandidateWindow(true, anchorRect, content, work, screenMargin, gap, &x, &y) &&
      !SidePlaceCandidateWindow(false, anchorRect, content, work, screenMargin, gap, &x, &y)) {
    x = preferredX;
    y = std::clamp(static_cast<int>(anchorRect.bottom) + gap, minY, maxY);
  }
  return {x, y, x + content.cx, y + content.cy};
}

void CCandidateWindow::ApplyWindowRegion(int width, int height, bool redraw) {
  if (!m_hwnd || width <= 0 || height <= 0) return;
  const LayoutSpec spec = ResolveLayoutSpec(m_style);
  const int radius = SnapGdiRadiusForDpi(
      Scale(m_style.skinLoaded && m_style.skinCornerRadius >= 0 ? m_style.skinCornerRadius
                                                                : spec.cornerRadius),
      m_dpi);
  const int w = std::max(1, width);
  const int h = std::max(1, height);
  if (radius <= 0) {
    SetWindowRgn(m_hwnd, nullptr, redraw ? TRUE : FALSE);
    return;
  }
  HRGN region = CreateRoundRectRgn(0, 0, w + 1, h + 1, radius, radius);
  if (region) {
    if (SetWindowRgn(m_hwnd, region, redraw ? TRUE : FALSE) == 0) {
      DeleteObject(region);
    }
  }
}

void CCandidateWindow::ApplyWindowRect(const RECT& rect) {
  m_targetWindowRect = rect;
  m_hasTargetWindowRect = true;
  float showProgress = 1.0f;
  if (m_showAnimationActive) {
    showProgress = EaseOutCubic(
        LinearAnimationProgress(GetTickCount64(), m_showAnimationStart, m_showAnimationDurationMs));
  }
  ApplyAnimatedWindowRect(showProgress);
}

void CCandidateWindow::ApplyAnimatedWindowRect(float showProgress) {
  if (!m_hwnd || !m_hasTargetWindowRect) return;
  RECT rect = m_targetWindowRect;
  if (m_showAnimationActive) {
    const int offsetY =
        static_cast<int>(std::lround(static_cast<float>(Scale(2)) * (1.0f - showProgress)));
    OffsetRect(&rect, 0, offsetY);
  }
  const int w = std::max<int>(1, rect.right - rect.left);
  const int h = std::max<int>(1, rect.bottom - rect.top);
  RECT current = {};
  const bool haveCurrentRect = GetWindowRect(m_hwnd, &current) != FALSE;
  const bool rectUnchanged = haveCurrentRect && EqualRect(&current, &rect);
  UINT flags = SWP_NOACTIVATE | SWP_NOOWNERZORDER;
  if (rectUnchanged) {
    flags |= SWP_NOMOVE | SWP_NOSIZE;
  }
  SetWindowPos(m_hwnd, m_style.candidateTopmost ? HWND_TOPMOST : HWND_NOTOPMOST,
               rect.left, rect.top, w, h, flags);
  if (!rectUnchanged) {
    ApplyWindowRegion(w, h, false);
  }
  UpdateShadowWindow();
}

void CCandidateWindow::ReleasePaintBuffer() {
  if (m_paintMemDc && m_paintOldBitmap) {
    SelectObject(m_paintMemDc, m_paintOldBitmap);
    m_paintOldBitmap = nullptr;
  }
  DeleteGdiObject(m_paintBitmap);
  if (m_paintMemDc) {
    DeleteDC(m_paintMemDc);
    m_paintMemDc = nullptr;
  }
  m_paintBitmap = nullptr;
  m_paintBufferW = 0;
  m_paintBufferH = 0;
}

void CCandidateWindow::ReleaseStaticPaintBuffer() {
  if (m_staticMemDc && m_staticOldBitmap) {
    SelectObject(m_staticMemDc, m_staticOldBitmap);
    m_staticOldBitmap = nullptr;
  }
  DeleteGdiObject(m_staticBitmap);
  if (m_staticMemDc) {
    DeleteDC(m_staticMemDc);
    m_staticMemDc = nullptr;
  }
  m_staticBitmap = nullptr;
  m_staticBufferW = 0;
  m_staticBufferH = 0;
  m_staticPaintDirty = true;
}

void CCandidateWindow::ReleaseItemPaintCaches() {
  for (auto& stateCaches : m_itemPaintCaches) {
    for (auto& cache : stateCaches) {
      cache.Release();
    }
  }
  m_itemPaintCaches.clear();
}

void CCandidateWindow::ReleaseItemPaintCacheAt(size_t index) {
  if (index >= m_itemPaintCaches.size()) return;
  for (auto& cache : m_itemPaintCaches[index]) {
    cache.Release();
  }
}

bool CCandidateWindow::EnsurePaintBuffer(HDC hdc, int w, int h) {
  if (!hdc || w <= 0 || h <= 0) return false;
  if (m_paintMemDc && m_paintBitmap && m_paintBufferW == w && m_paintBufferH == h) return true;

  ReleasePaintBuffer();
  m_paintMemDc = CreateCompatibleDC(hdc);
  if (!m_paintMemDc) return false;
  m_paintBitmap = CreateCompatibleBitmap(hdc, w, h);
  if (!m_paintBitmap) {
    ReleasePaintBuffer();
    return false;
  }
  m_paintOldBitmap = static_cast<HBITMAP>(SelectObject(m_paintMemDc, m_paintBitmap));
  m_paintBufferW = w;
  m_paintBufferH = h;
  return true;
}

bool CCandidateWindow::EnsureStaticPaintBuffer(HDC hdc, int w, int h) {
  if (!hdc || w <= 0 || h <= 0) return false;
  if (m_staticMemDc && m_staticBitmap && m_staticBufferW == w && m_staticBufferH == h) {
    return true;
  }

  ReleaseStaticPaintBuffer();
  m_staticMemDc = CreateCompatibleDC(hdc);
  if (!m_staticMemDc) return false;
  m_staticBitmap = CreateCompatibleBitmap(hdc, w, h);
  if (!m_staticBitmap) {
    ReleaseStaticPaintBuffer();
    return false;
  }
  m_staticOldBitmap = static_cast<HBITMAP>(SelectObject(m_staticMemDc, m_staticBitmap));
  m_staticBufferW = w;
  m_staticBufferH = h;
  m_staticPaintDirty = true;
  return true;
}

void CCandidateWindow::PaintFull(HDC memDc, const RECT& client, const RECT* dirtyRect) {
  // 每次整窗重绘按当前 client 宽度重算卡片位置，避免横向换行/皮肤 DPI 后与 m_itemRects 脱节。
  // m_needsMeasure：SetStyle/DPI 变更等场景下显式标记需要重测布局。
  const SIZE currentSize = {client.right - client.left, client.bottom - client.top};
  if (!dirtyRect && (m_needsMeasure || m_itemRects.empty() ||
                     currentSize.cx != m_measuredClientSize.cx ||
                     currentSize.cy != m_measuredClientSize.cy)) {
    m_measuredClientSize = MeasureClientSize(currentSize.cx, &m_itemRects);
    m_needsMeasure = false;
    m_lastLayoutHorizontal = m_style.candidateHorizontal;
    m_lastLayoutVariant = m_style.candidateLayoutVariant;
  }

  CandidateColors colors = ResolveColors(m_style);
  // Keep retained candidates visually stable while an asynchronous lookup is
  // pending. Their interaction state is still synchronized through
  // m_interactive, but blending the whole palette with the window background
  // made the selected first row flash pale on every key press (especially in
  // skins that use white selected text).
  const LayoutSpec spec = ResolveLayoutSpec(m_style);

  // Apply skin corner radius overrides
  const int strokeWidth = StrokeWidthForDpi(m_dpi);
  const int radius = SnapGdiRadiusForDpi(
      Scale(m_style.skinLoaded && m_style.skinCornerRadius >= 0 ? m_style.skinCornerRadius
                                                                : spec.cornerRadius),
      m_dpi);
  const int headerRadius = SnapGdiRadiusForDpi(
      Scale(m_style.skinLoaded && m_style.skinHeaderCornerRadius >= 0
                ? m_style.skinHeaderCornerRadius
                : spec.itemRadius),
      m_dpi);
  const int itemRadius = SnapGdiRadiusForDpi(
      Scale(m_style.skinLoaded && m_style.skinRowCornerRadius >= 0 ? m_style.skinRowCornerRadius
                                                                   : spec.itemRadius),
      m_dpi);
  const int badgeRadius = SnapGdiRadiusForDpi(
      Scale(m_style.skinLoaded && m_style.skinBadgeCornerRadius >= 0
                ? m_style.skinBadgeCornerRadius
                : spec.badgeRadius),
      m_dpi);

  const int outerPadX = Scale(spec.outerPadX);
  const int outerPadY = Scale(spec.outerPadY);
  const int headerPadX = Scale(spec.headerPadX);
  const int headerPadY = Scale(spec.headerPadY);
  const int headerGap = Scale(spec.headerGap);
  const int itemPadX = Scale(spec.itemPadX);
  const int itemPadY = Scale(spec.itemPadY);
  int labelWidth = Scale(spec.labelWidth);
  const int labelGap = Scale(spec.labelGap);
  const int commentGap = Scale(spec.commentGap);

  const bool isHorizontal = m_style.candidateHorizontal;
  const bool clipboardMode = !isHorizontal && HasClipboardCandidateItems(m_clipboardItems);
  if (clipboardMode) labelWidth = Scale(22);
  int compact1 = 0;
  if (isHorizontal) {
    compact1 = HorizontalCompactDeltaForDpi(m_style, m_dpi);
  }
  const int compact2 = isHorizontal ? std::max(Scale(2), compact1 * 2) : 0;
  const int hOuterPadX = isHorizontal ? std::max(0, outerPadX - compact1) : outerPadX;
  const int hOuterPadY = isHorizontal ? std::max(0, outerPadY - compact1) : outerPadY;
  const int hItemPadX = isHorizontal ? std::max(0, itemPadX - compact1) : itemPadX;
  const int hItemPadY = isHorizontal ? std::max(0, itemPadY - compact1) : itemPadY;
  const int hBadgeHeight = isHorizontal ? std::max(Scale(16), Scale(22) - compact2) : Scale(22);
  const int hBadgeGap = isHorizontal ? std::max(0, Scale(4) - compact1) : Scale(4);
  const bool showHorizontalPageBadge = isHorizontal && m_totalPages > 1;

  auto drawStaticLayer = [&](HDC targetDc) {
    FillBackgroundClippedToRoundRect(targetDc, client, colors, radius);
    const COLORREF borderColor = AlphaBlendColor(colors.border, colors.windowBg, colors.borderOpacity);
    DrawRoundRectBorderCached(targetDc, client, borderColor, radius, strokeWidth);

    if (!isHorizontal) {
      const bool showPageBadge = m_totalPages > 1;
      std::wstring pageText;
      int pageBadgeWidth = 0;
      if (showPageBadge) {
        pageText = CandidatePageIndicatorText(m_pageIndex, m_totalPages);
        const SIZE pageTextSize = MeasureSingleLineCached(targetDc, m_metaFont, pageText);
        pageBadgeWidth = std::max(Scale(kHorizontalPageBadgeMinWidth),
                                   static_cast<int>(pageTextSize.cx) + Scale(12));
      }
      const SIZE titleSize = MeasureSingleLineCached(targetDc, m_titleFont, m_title);
      const int headerHeight = std::max(Scale(30), static_cast<int>(titleSize.cy) + headerPadY * 2);

      RECT headerRect = {outerPadX, outerPadY, client.right - outerPadX, outerPadY + headerHeight};
      FillHeaderBackgroundClippedToRoundRect(targetDc, headerRect, colors, headerRadius);
      DrawRoundRectBorderCached(targetDc, headerRect, borderColor, headerRadius, strokeWidth);

      RECT pageRect = {};
      if (showPageBadge) {
        pageRect = {headerRect.right - headerPadX - pageBadgeWidth, headerRect.top + headerPadY / 2,
                    headerRect.right - headerPadX, headerRect.bottom - headerPadY / 2};
        FillBorderedRoundRectCached(targetDc, pageRect, colors.badgeBg, colors.badgeBorder, badgeRadius,
                                    strokeWidth);
        DrawTextBlock(targetDc, m_metaFont, colors.badgeText, pageText, pageRect,
                      DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
      }

      const int modeChipGap = Scale(4);
      const int modeChipPadX = Scale(m_style.skinLoaded ? 6 : 7);
      const int modeChipHeight =
          std::max<int>(Scale(17), static_cast<int>(headerRect.bottom - headerRect.top - headerPadY * 2));
      int chipRightBound = showPageBadge ? pageRect.left - Scale(8) : headerRect.right - headerPadX;
      int modeChipX = chipRightBound;
      for (size_t idx = m_modeTags.size(); idx > 0; --idx) {
        const std::wstring& tag = m_modeTags[idx - 1];
        if (tag.empty()) continue;
        const SIZE tagSize = MeasureSingleLine(targetDc, m_chipFont, tag, m_dpi);
        const int chipWidth = std::max(Scale(30), static_cast<int>(tagSize.cx) + modeChipPadX * 2);
        modeChipX -= chipWidth;
        RECT chipRect = {modeChipX, headerRect.top + (headerRect.bottom - headerRect.top - modeChipHeight) / 2,
                         modeChipX + chipWidth,
                         headerRect.top + (headerRect.bottom - headerRect.top - modeChipHeight) / 2 +
                             modeChipHeight};
        const bool active =
            tag == L"\u4e2d" || tag == L"\u5168\u89d2" || tag == L"\u4e2d\u6807" ||
            tag == L"\u53cc\u62fc" || tag == L"\u6a21\u7cca";
        const COLORREF chipFill = active ? colors.chipActiveBg : colors.chipBg;
        const COLORREF chipBorder = active ? colors.chipActiveBorder : colors.chipBorder;
        const COLORREF chipText = active ? colors.chipActiveText : colors.chipText;
        FillBorderedRoundRectCached(targetDc, chipRect, chipFill, chipBorder, badgeRadius,
                                    strokeWidth);
        DrawTextBlock(targetDc, m_chipFont, chipText, tag, chipRect,
                      DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
        modeChipX -= modeChipGap;
      }

      RECT titleRect = {
          headerRect.left + headerPadX,
          headerRect.top + headerPadY,
          std::max<int>(static_cast<int>(headerRect.left + headerPadX), modeChipX - Scale(6)),
          headerRect.bottom - headerPadY};
      DrawTextBlock(targetDc, m_titleFont, colors.text, m_title, titleRect,
                    DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);

      if (m_style.candidateLayoutVariant != SrfCandidateLayoutVariant::Card) {
        for (size_t i = 0; i + 1 < m_itemRects.size(); ++i) {
          const int divY = (m_itemRects[i].bottom + m_itemRects[i + 1].top) / 2;
          DrawDivider(targetDc, divY, outerPadX + Scale(8), client.right - outerPadX - Scale(8),
                      colors, strokeWidth);
        }
      }
      if (clipboardMode) {
        for (size_t i = 0; i + 1 < m_itemRects.size() && i + 1 < m_pinnedItems.size(); ++i) {
          if (!m_pinnedItems[i] || m_pinnedItems[i + 1]) continue;
          const int centerY = (m_itemRects[i].bottom + m_itemRects[i + 1].top) / 2;
          RECT recentRect = {outerPadX + Scale(10), centerY - Scale(8),
                             outerPadX + Scale(48), centerY + Scale(8)};
          DrawTextBlock(targetDc, m_metaFont, colors.mutedText, L"\u6700\u8fd1", recentRect,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
          DrawDivider(targetDc, centerY, recentRect.right + Scale(6),
                      client.right - outerPadX - Scale(8), colors, strokeWidth);
          break;
        }

        const int footerHeight = Scale(28);
        RECT footerRect = {outerPadX + Scale(8), client.bottom - outerPadY - footerHeight,
                           client.right - outerPadX - Scale(8), client.bottom - outerPadY};
        DrawDivider(targetDc, footerRect.top - Scale(4), footerRect.left, footerRect.right,
                    colors, strokeWidth);
        DrawTextBlock(targetDc, m_metaFont, colors.mutedText,
                      L"\u2191\u2193 \u9009\u62e9   PgUp/PgDn \u7ffb\u9875   Enter \u7c98\u8d34   Esc \u5173\u95ed",
                      footerRect, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS,
                      m_dpi);
      }
    } else {
      const std::wstring horizontalPageText =
          CandidatePageIndicatorText(m_pageIndex, m_totalPages);
      const SIZE horizontalPageTextSize =
          showHorizontalPageBadge
              ? MeasureSingleLineCached(targetDc, m_metaFont, horizontalPageText)
              : SIZE{};
      const int horizontalPageBadgeWidth =
          showHorizontalPageBadge
              ? std::max(Scale(kHorizontalPageBadgeMinWidth),
                         static_cast<int>(horizontalPageTextSize.cx) + Scale(12))
              : 0;
      const int horizontalPageBadgeHeight =
          showHorizontalPageBadge
              ? std::max(hBadgeHeight, static_cast<int>(horizontalPageTextSize.cy) +
                                           Scale(kHorizontalPageBadgePaddingY))
              : 0;
      // Horizontal mode intentionally stays compact, but a small trailing
      // page badge makes additional candidates discoverable without adding a
      // second row or a footer.
      if (showHorizontalPageBadge) {
        RECT pageRect = {client.right - hOuterPadX - horizontalPageBadgeWidth,
                         hOuterPadY, client.right - hOuterPadX,
                         hOuterPadY + horizontalPageBadgeHeight};
        FillBorderedRoundRectCached(targetDc, pageRect, colors.badgeBg, colors.badgeBorder,
                                    badgeRadius, strokeWidth);
        DrawTextBlock(targetDc, m_metaFont, colors.badgeText, horizontalPageText, pageRect,
                      DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
      }
      const ULONGLONG now = GetTickCount64();
      const bool showPendingIndicator =
          m_pendingVisual && m_pendingVisualSince != 0 && now >= m_pendingVisualSince &&
          now - m_pendingVisualSince >= kCandidatePendingIndicatorDelayMs;
      if (showPendingIndicator) {
        // A stale/pending snapshot remains readable and non-interactive. Show
        // only a tiny accent dot after the async state changes; avoid dimming
        // the whole strip and making fast typing feel sluggish.
        const int dot = std::max(Scale(3), strokeWidth + 1);
        const int right = client.right - hOuterPadX -
                          (showHorizontalPageBadge ? horizontalPageBadgeWidth + Scale(6) : 0);
        RECT dotRect = {right - dot, hOuterPadY + Scale(2), right,
                        hOuterPadY + Scale(2) + dot};
        FillSolidRect(targetDc, dotRect, colors.selectedBorder);
      }
    }
  };

  const int clientW = client.right - client.left;
  const int clientH = client.bottom - client.top;
  const bool staticReady = EnsureStaticPaintBuffer(memDc, clientW, clientH);
  if (staticReady) {
    if (m_staticPaintDirty) {
      drawStaticLayer(m_staticMemDc);
      m_staticPaintDirty = false;
    }
    const RECT copyRect = dirtyRect ? *dirtyRect : client;
    BitBlt(memDc, copyRect.left, copyRect.top, copyRect.right - copyRect.left,
           copyRect.bottom - copyRect.top, m_staticMemDc, copyRect.left, copyRect.top, SRCCOPY);
  } else {
    drawStaticLayer(memDc);
  }
  if (m_displayItems.size() != m_items.size()) RebuildDisplayItems();
  const std::vector<std::wstring>& displayItems = m_displayItems;

  if (m_itemPaintCaches.size() < m_itemRects.size()) {
    m_itemPaintCaches.resize(m_itemRects.size());
  }

  const ULONGLONG animationNow = GetTickCount64();
  const float selectionProgress =
      m_selectionAnimationActive
          ? EaseOutCubic(LinearAnimationProgress(animationNow, m_selectionAnimationStart,
                                                 m_selectionAnimationDurationMs))
          : 1.0f;
  const float hoverProgress =
      m_hoverAnimationActive ? EaseOutCubic(LinearAnimationProgress(
                                   animationNow, m_hoverAnimationStart, m_hoverAnimationDurationMs))
                             : 1.0f;
  const float pressProgress =
      m_pressAnimationActive ? EaseOutCubic(LinearAnimationProgress(
                                   animationNow, m_pressAnimationStart, m_pressAnimationDurationMs))
                             : 1.0f;
  const float pageProgress =
      m_pageAnimationActive ? EaseOutCubic(LinearAnimationProgress(
                                  animationNow, m_pageAnimationStart, m_pageAnimationDurationMs))
                            : 1.0f;
  const int pageOffsetX = m_pageAnimationActive
                              ? static_cast<int>(std::lround(m_pageAnimationDirection * Scale(6) *
                                                             (1.0f - pageProgress)))
                              : 0;
  const bool anyItemAnimation = m_selectionAnimationActive || m_hoverAnimationActive ||
                                m_pressAnimationActive || m_pageAnimationActive;
  const COLORREF accentColor =
      colors.selectedOutline != CLR_INVALID ? colors.selectedOutline : colors.selectedBorder;
  const int selectedAccentWidth = Scale(m_style.skinLoaded && m_style.skinSelectedAccentWidth >= 0
                                            ? m_style.skinSelectedAccentWidth
                                            : 3);
  const float selectedRingOpacity = (m_style.skinLoaded && m_style.skinSelectedRingOpacity >= 0.0f)
                                        ? m_style.skinSelectedRingOpacity
                                        : 0.0f;
  const std::wstring selectedIndicator =
      (m_style.skinLoaded && !m_style.skinSelectedIndicator.empty())
          ? m_style.skinSelectedIndicator
          : std::wstring(DefaultCandidateSelectedIndicator(isHorizontal));

  for (size_t i = 0; i < m_itemRects.size(); ++i) {
    if (dirtyRect) {
      RECT tmp;
      if (!IntersectRect(&tmp, &m_itemRects[i], dirtyRect)) continue;
    }
    const bool isSelected = static_cast<UINT>(i) == m_selectedInPage;
    const bool isPressed = static_cast<int>(i) == m_pressedIndex;
    const bool isHot = static_cast<int>(i) == m_hotIndex;
    const float selectionWeight =
        m_selectionAnimationActive
            ? IndexTransitionWeight(i, m_selectionAnimationFrom, m_selectionAnimationTo,
                                    selectionProgress)
            : (isSelected ? 1.0f : 0.0f);
    const float hoverWeight =
        m_hoverAnimationActive
            ? IndexTransitionWeight(i, m_hoverAnimationFrom, m_hoverAnimationTo, hoverProgress)
            : (isHot ? 1.0f : 0.0f);
    const float pressWeight =
        m_pressAnimationActive
            ? IndexTransitionWeight(i, m_pressAnimationFrom, m_pressAnimationTo, pressProgress)
            : (isPressed ? 1.0f : 0.0f);
    COLORREF itemFill = AlphaBlendColor(colors.hoverBg, colors.itemBg, hoverWeight);
    itemFill = AlphaBlendColor(colors.selectedBg, itemFill, selectionWeight);
    itemFill = AlphaBlendColor(colors.pressedBg, itemFill, pressWeight);
    COLORREF normalBorder = isHorizontal ? colors.itemBg : colors.itemBorder;
    COLORREF itemBorder = AlphaBlendColor(colors.hoverBorder, normalBorder, hoverWeight);
    itemBorder = AlphaBlendColor(colors.selectedBorder, itemBorder, selectionWeight);
    itemBorder = AlphaBlendColor(colors.pressedBorder, itemBorder, pressWeight);
    COLORREF textColor = AlphaBlendColor(colors.selectedText, colors.text, selectionWeight);
    COLORREF mutedColor =
        AlphaBlendColor(colors.selectedMutedText, colors.mutedText, selectionWeight);
    const COLORREF neutralBadgeFill = AlphaBlendColor(colors.badgeBg, colors.itemBg, 0.72f);
    const COLORREF neutralBadgeBorder = AlphaBlendColor(colors.badgeBorder, colors.itemBg, 0.55f);
    const COLORREF selectedBadgeFill =
        m_style.skinLoaded ? accentColor : AlphaBlendColor(accentColor, colors.selectedBg, 0.14f);
    const COLORREF selectedBadgeBorder =
        m_style.skinLoaded ? accentColor : AlphaBlendColor(accentColor, colors.selectedBg, 0.36f);
    COLORREF badgeFill = AlphaBlendColor(selectedBadgeFill, neutralBadgeFill, selectionWeight);
    COLORREF badgeBorder =
        AlphaBlendColor(selectedBadgeBorder, neutralBadgeBorder, selectionWeight);
    COLORREF badgeText = AlphaBlendColor(colors.selectedText, colors.badgeText, selectionWeight);
    if (m_pageAnimationActive) {
      itemFill = AlphaBlendColor(itemFill, colors.windowBg, pageProgress);
      itemBorder = AlphaBlendColor(itemBorder, colors.windowBg, pageProgress);
      textColor = AlphaBlendColor(textColor, colors.windowBg, pageProgress);
      mutedColor = AlphaBlendColor(mutedColor, colors.windowBg, pageProgress);
      badgeFill = AlphaBlendColor(badgeFill, colors.windowBg, pageProgress);
      badgeBorder = AlphaBlendColor(badgeBorder, colors.windowBg, pageProgress);
      badgeText = AlphaBlendColor(badgeText, colors.windowBg, pageProgress);
    }
    const COLORREF visualAccentColor =
        m_pageAnimationActive ? AlphaBlendColor(accentColor, colors.windowBg, pageProgress)
                              : accentColor;
    HFONT bodyFontForState = isSelected && m_bodyStrongFont ? m_bodyStrongFont : m_bodyFont;

    RECT srcRect = m_itemRects[i];
    OffsetRect(&srcRect, pageOffsetX, 0);
    const int itemW = srcRect.right - srcRect.left;
    const int itemH = srcRect.bottom - srcRect.top;

    const bool canUseCache = !anyItemAnimation;
    CandidateItemPaintCache& cache =
        m_itemPaintCaches[i][CandidateItemPaintStateSlot(isSelected, isHot, isPressed)];
    const bool cacheHit = canUseCache && cache.valid && cache.itemIndex == i &&
                          cache.w == itemW && cache.h == itemH &&
                          cache.selected == isSelected && cache.hot == isHot &&
                          cache.pressed == isPressed;

    if (cacheHit) {
      BitBlt(memDc, srcRect.left, srcRect.top, itemW, itemH, cache.memDc, 0, 0, SRCCOPY);
      continue;
    }

    HDC targetDc = memDc;
    int offsetX = srcRect.left;
    int offsetY = srcRect.top;
    bool willCache = canUseCache;
    if (willCache) {
      if (!cache.Ensure(memDc, itemW, itemH)) {
        willCache = false;
      } else {
        BitBlt(cache.memDc, 0, 0, itemW, itemH, memDc, srcRect.left, srcRect.top, SRCCOPY);
        targetDc = cache.memDc;
        offsetX = 0;
        offsetY = 0;
      }
    }

    RECT rect = {offsetX, offsetY, offsetX + itemW, offsetY + itemH};

    if (m_style.candidateHorizontal) {
      const bool drawHorizontalCard =
          m_style.candidateLayoutVariant == SrfCandidateLayoutVariant::Card ||
          selectionWeight > 0.001f || pressWeight > 0.001f || hoverWeight > 0.001f;
      if (drawHorizontalCard) {
        FillBorderedRoundRectCached(targetDc, rect, itemFill, itemBorder,
                                    std::max(Scale(4), itemRadius / 2), strokeWidth);
      }
      if (!m_selectionAnimationActive && selectionWeight > 0.001f &&
          selectedIndicator == L"bottom_bar" && selectedAccentWidth > 0) {
        const int accentH = std::min(selectedAccentWidth, std::max(1, itemH / 3));
        RECT accentRect = {rect.left + Scale(6), rect.bottom - accentH - Scale(1),
                           rect.right - Scale(6), rect.bottom - Scale(1)};
        FillSolidRect(targetDc, accentRect, visualAccentColor);
      } else if (!m_selectionAnimationActive && selectionWeight > 0.001f &&
                 selectedIndicator == L"left_bar" && selectedAccentWidth > 0) {
        const int accentW = std::min(selectedAccentWidth, std::max(1, itemW / 3));
        RECT accentRect = {rect.left + Scale(2), rect.top + Scale(4),
                           rect.left + Scale(2) + accentW, rect.bottom - Scale(4)};
        FillSolidRect(targetDc, accentRect, visualAccentColor);
      } else if (!m_selectionAnimationActive && selectionWeight > 0.001f &&
                 selectedIndicator == L"outline") {
        DrawRoundRectBorderCached(targetDc, rect, visualAccentColor,
                                  std::max(Scale(4), itemRadius / 2), strokeWidth);
      }
      if (selectionWeight > 0.001f && selectedRingOpacity > 0.001f) {
        const COLORREF ringColor =
            AlphaBlendColor(visualAccentColor, itemFill, selectedRingOpacity * selectionWeight);
        RECT ringRect = {rect.left + strokeWidth, rect.top + strokeWidth,
                         rect.right - strokeWidth, rect.bottom - strokeWidth};
        DrawRoundRectBorderCached(targetDc, ringRect, ringColor,
                                  std::max(Scale(3), itemRadius / 2 - strokeWidth), strokeWidth);
      }

      const std::wstring labelText = i < m_labels.size() ? m_labels[i] : std::wstring();
      const SIZE labelSize = MeasureLabelCached(targetDc, labelText, i);
      const int labelW = std::max(Scale(10), static_cast<int>(labelSize.cx));
      COLORREF inlineLabelColor = AlphaBlendColor(accentColor, colors.mutedText, selectionWeight);
      if (m_pageAnimationActive) {
        inlineLabelColor = AlphaBlendColor(inlineLabelColor, colors.windowBg, pageProgress);
      }
      const int contentLeft = rect.left + hItemPadX;
      const int contentRight = std::max(contentLeft, static_cast<int>(rect.right) - hItemPadX);
      const int contentTop = rect.top + hItemPadY;
      const int contentBottom = std::max(contentTop, static_cast<int>(rect.bottom) - hItemPadY);
      const int labelRight = std::min(contentRight, contentLeft + labelW);
      const int textLeft = std::min(contentRight, labelRight + labelGap);
      const bool hasComment = isSelected && i < m_comments.size() && !m_comments[i].empty();

      if (hasComment) {
        const SIZE bodySize = MeasureSingleLine(targetDc, bodyFontForState, displayItems[i], m_dpi);
        const SIZE commentSize = MeasureSingleLine(targetDc, m_metaFont, m_comments[i], m_dpi);
        const int mainH =
            std::max({Scale(18), static_cast<int>(labelSize.cy), static_cast<int>(bodySize.cy)});
        const int commentH = std::max(Scale(12), static_cast<int>(commentSize.cy));
        const int stackH = mainH + commentGap + commentH;
        const int availableH = contentBottom - contentTop;
        const int stackTop = contentTop + std::max(0, (availableH - stackH) / 2);
        const int mainTop = std::min(contentBottom, stackTop + commentH + commentGap);
        RECT commentRect = {textLeft, stackTop, contentRight,
                            std::min(contentBottom, stackTop + commentH)};
        RECT labelRect = {contentLeft, mainTop, labelRight, contentBottom};
        RECT textRect = {textLeft, mainTop, contentRight, contentBottom};
        DrawTextBlock(targetDc, m_metaFont, mutedColor, m_comments[i], commentRect,
                      DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
        DrawTextBlock(targetDc, m_labelFont, inlineLabelColor, labelText, labelRect,
                      DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
        DrawTextBlock(targetDc, bodyFontForState, textColor, displayItems[i], textRect,
                      DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
      } else {
        RECT labelRect = {contentLeft, contentTop, labelRight, contentBottom};
        RECT textRect = {textLeft, contentTop, contentRight, contentBottom};
        DrawTextBlock(targetDc, m_labelFont, inlineLabelColor, labelText, labelRect,
                      DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
        DrawTextBlock(targetDc, bodyFontForState, textColor, displayItems[i], textRect,
                      DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
      }
    } else {
      const bool drawVerticalCard =
          (!clipboardMode && m_style.candidateLayoutVariant == SrfCandidateLayoutVariant::Card) ||
          selectionWeight > 0.001f || pressWeight > 0.001f || hoverWeight > 0.001f;
      if (drawVerticalCard) {
        FillBorderedRoundRectCached(targetDc, rect, itemFill, itemBorder, itemRadius, strokeWidth);
      }

      if (!m_selectionAnimationActive && selectionWeight > 0.001f &&
          selectedIndicator == L"left_bar" && selectedAccentWidth > 0) {
        const int accentW = selectedAccentWidth;
        const int marginY = Scale(4);
        RECT accentRect = {rect.left + Scale(3), rect.top + marginY,
                           rect.left + Scale(3) + accentW, rect.bottom - marginY};
        FillSolidRect(targetDc, accentRect, visualAccentColor);
      } else if (!m_selectionAnimationActive && selectionWeight > 0.001f &&
                 selectedIndicator == L"outline") {
        DrawRoundRectBorderCached(targetDc, rect, visualAccentColor, itemRadius, strokeWidth);
      }
      if (selectionWeight > 0.001f && selectedRingOpacity > 0.001f) {
        const COLORREF ringColor =
            AlphaBlendColor(visualAccentColor, itemFill, selectedRingOpacity * selectionWeight);
        RECT ringRect = {rect.left + strokeWidth, rect.top + strokeWidth,
                         rect.right - strokeWidth, rect.bottom - strokeWidth};
        DrawRoundRectBorderCached(targetDc, ringRect, ringColor,
                                  std::max(Scale(3), itemRadius - strokeWidth), strokeWidth);
      }
      const int badgeH = clipboardMode ? Scale(19) : std::max(Scale(22), labelWidth);
      const int badgeW = clipboardMode ? Scale(20) : labelWidth;
      RECT badgeRect = {rect.left + itemPadX, rect.top + itemPadY,
                        rect.left + itemPadX + badgeW, rect.top + itemPadY + badgeH};
      badgeRect.bottom = std::min(badgeRect.bottom, rect.bottom - itemPadY);
      FillBorderedRoundRectCached(targetDc, badgeRect, badgeFill, badgeBorder, badgeRadius,
                                  strokeWidth);
      DrawTextBlock(targetDc, m_labelFont, badgeText,
                    i < m_labels.size() ? m_labels[i] : std::wstring(), badgeRect,
                    DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);

      RECT textRect = {badgeRect.right + labelGap, rect.top + itemPadY, rect.right - itemPadX,
                       rect.bottom - itemPadY};
      const ClipboardCommentParts clipboardComment =
          i < m_clipboardCommentParts.size() ? m_clipboardCommentParts[i]
                                             : ClipboardCommentParts{};
      int commentHeight = 0;
      if (i < m_comments.size() && !m_comments[i].empty() &&
          (clipboardMode || isSelected)) {
        if (clipboardMode) {
          const SIZE detailSize =
              MeasureSingleLine(targetDc, m_metaFont, clipboardComment.detail, m_dpi);
          const SIZE typeSize =
              MeasureSingleLine(targetDc, m_chipFont, clipboardComment.type, m_dpi);
          commentHeight = std::max<int>(Scale(16), std::max(detailSize.cy, typeSize.cy));
        } else {
          commentHeight = MeasureWrappedHeight(targetDc, m_metaFont, m_comments[i],
                                               textRect.right - textRect.left, m_dpi);
        }
      }
      RECT mainRect = textRect;
      if (commentHeight > 0) {
        mainRect.bottom = std::max(mainRect.top, textRect.bottom - commentHeight - commentGap);
      }
      DrawTextBlock(targetDc, bodyFontForState, textColor, displayItems[i], mainRect,
                    DT_LEFT | DT_TOP | DT_WORDBREAK, m_dpi);

      if (commentHeight > 0) {
        RECT commentRect = {textRect.left, mainRect.bottom + commentGap, textRect.right,
                            textRect.bottom};
        if (clipboardMode) {
          int detailRight = commentRect.right;
          if (!clipboardComment.type.empty()) {
            const SIZE typeSize =
                MeasureSingleLine(targetDc, m_chipFont, clipboardComment.type, m_dpi);
            const int chipWidth = std::max(Scale(34), static_cast<int>(typeSize.cx) + Scale(12));
            RECT typeRect = {std::max(commentRect.left, commentRect.right - chipWidth),
                             commentRect.top, commentRect.right, commentRect.bottom};
            FillBorderedRoundRectCached(targetDc, typeRect,
                                        isSelected ? colors.chipActiveBg : colors.chipBg,
                                        isSelected ? colors.chipActiveBorder : colors.chipBorder,
                                        badgeRadius, strokeWidth);
            DrawTextBlock(targetDc, m_chipFont,
                          isSelected ? colors.chipActiveText : colors.chipText,
                          clipboardComment.type, typeRect,
                          DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
            detailRight = typeRect.left - Scale(8);
          }
          RECT detailRect = {commentRect.left, commentRect.top,
                             std::max<int>(static_cast<int>(commentRect.left), detailRight),
                             commentRect.bottom};
          DrawTextBlock(targetDc, m_metaFont, mutedColor, clipboardComment.detail, detailRect,
                        DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
        } else {
          DrawTextBlock(targetDc, m_metaFont, mutedColor, m_comments[i], commentRect,
                        DT_LEFT | DT_TOP | DT_WORDBREAK, m_dpi);
        }
      }
    }

    if (willCache) {
      cache.itemIndex = i;
      cache.selected = isSelected;
      cache.hot = isHot;
      cache.pressed = isPressed;
      cache.valid = true;
      BitBlt(memDc, srcRect.left, srcRect.top, itemW, itemH, cache.memDc, 0, 0, SRCCOPY);
    }
  }

  if (m_selectionAnimationActive && m_selectionAnimationFrom >= 0 && m_selectionAnimationTo >= 0 &&
      static_cast<size_t>(m_selectionAnimationFrom) < m_itemRects.size() &&
      static_cast<size_t>(m_selectionAnimationTo) < m_itemRects.size() && selectedAccentWidth > 0 &&
      selectedIndicator != L"none") {
    const RECT indicatorRect = InterpolateRect(
        m_itemRects[static_cast<size_t>(m_selectionAnimationFrom)],
        m_itemRects[static_cast<size_t>(m_selectionAnimationTo)], selectionProgress);
    if (selectedIndicator == L"bottom_bar") {
      const int accentH =
          std::min(selectedAccentWidth,
                   std::max(1, static_cast<int>(indicatorRect.bottom - indicatorRect.top) / 3));
      RECT accentRect = {indicatorRect.left + Scale(6), indicatorRect.bottom - accentH - Scale(1),
                         indicatorRect.right - Scale(6), indicatorRect.bottom - Scale(1)};
      FillSolidRect(memDc, accentRect, accentColor);
    } else if (selectedIndicator == L"left_bar") {
      const int accentW =
          std::min(selectedAccentWidth,
                   std::max(1, static_cast<int>(indicatorRect.right - indicatorRect.left) / 3));
      const int insetX = isHorizontal ? Scale(2) : Scale(3);
      RECT accentRect = {indicatorRect.left + insetX, indicatorRect.top + Scale(4),
                         indicatorRect.left + insetX + accentW, indicatorRect.bottom - Scale(4)};
      FillSolidRect(memDc, accentRect, accentColor);
    } else if (selectedIndicator == L"outline") {
      DrawRoundRectBorderCached(memDc, indicatorRect, accentColor,
                                isHorizontal ? std::max(Scale(4), itemRadius / 2) : itemRadius,
                                strokeWidth);
    }
  }

  if (m_pinMenuVisible && m_pinMenuRect.right > m_pinMenuRect.left &&
      m_pinMenuRect.bottom > m_pinMenuRect.top) {
    const COLORREF borderColor = AlphaBlendColor(colors.border, colors.windowBg, colors.borderOpacity);
    const int menuRadius = std::max(Scale(4), itemRadius / 2);
    FillBorderedRoundRectCached(memDc, m_pinMenuRect, colors.headerBg, borderColor, menuRadius,
                                strokeWidth);

    auto drawPinMenuButton = [&](const RECT& rect, int command, const wchar_t* text) {
      const bool enabled = PinMenuCommandEnabled(command);
      const bool hot = enabled && m_pinMenuHotCommand == command;
      const bool pressed = m_pinMenuPressedCommand == command;
      const COLORREF fill =
          !enabled ? AlphaBlendColor(colors.itemBg, colors.windowBg, 0.55f)
                   : (pressed ? colors.pressedBg : (hot ? colors.hoverBg : colors.itemBg));
      const COLORREF border =
          !enabled ? AlphaBlendColor(colors.itemBorder, colors.windowBg, 0.45f)
                   : (pressed ? colors.pressedBorder
                              : (hot ? colors.hoverBorder : colors.itemBorder));
      const COLORREF textColor =
          !enabled ? AlphaBlendColor(colors.mutedText, colors.windowBg, 0.55f)
                   : (pressed ? colors.selectedText : colors.text);
      FillBorderedRoundRectCached(memDc, rect, fill, border,
                                  std::max(Scale(4), menuRadius - Scale(1)), strokeWidth);
      DrawTextBlock(memDc, m_bodyFont, textColor, text, rect,
                    DT_CENTER | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS, m_dpi);
    };

    drawPinMenuButton(m_pinMenuPinRect, kPinMenuCommandPin, L"\u56fa\u5b9a\u7f6e\u9876");
    drawPinMenuButton(m_pinMenuUnpinRect, kPinMenuCommandUnpin, L"\u53d6\u6d88\u56fa\u5b9a");
    drawPinMenuButton(m_pinMenuRemoveRect, kPinMenuCommandRemove, L"\u5220\u7528\u6237\u8bcd");
    drawPinMenuButton(m_pinMenuBlockRect, kPinMenuCommandBlock, L"\u5c4f\u853d\u8be5\u8bcd");
    drawPinMenuButton(m_pinMenuSourceRect, kPinMenuCommandSource, L"\u67e5\u770b\u6765\u6e90");
  }
}

void CCandidateWindow::Paint(HDC hdc, const RECT& paintRect) {
  const ULONGLONG paintStart = GetTickCount64();
  ++m_paintCount;
  RECT client = {};
  GetClientRect(m_hwnd, &client);
  const int w = client.right - client.left;
  const int h = client.bottom - client.top;
  const bool hadBuffer = m_paintMemDc && m_paintBitmap && m_paintBufferW == w && m_paintBufferH == h;
  bool fullPaint = false;
  if (!EnsurePaintBuffer(hdc, w, h)) {
    fullPaint = true;
    ++m_fullPaintCount;
    PaintFull(hdc, client);
    m_layoutDirty = false;
    m_fullPaintDirty = false;
    const unsigned long long fullPct =
        m_paintCount == 0 ? 0 : (m_fullPaintCount * 100 / m_paintCount);
    wchar_t line[224] = {};
    swprintf_s(line,
               L"stage=CandidateWindow/Paint elapsed_ms=%llu full=1 direct=1 paint_count=%llu full_count=%llu full_pct=%llu",
               static_cast<unsigned long long>(GetTickCount64() - paintStart), m_paintCount,
               m_fullPaintCount, fullPct);
    SrfTsfPerfLog(L"perf", line);
    return;
  }
  if (m_layoutDirty || m_fullPaintDirty || !hadBuffer) {
    fullPaint = true;
    ++m_fullPaintCount;
    PaintFull(m_paintMemDc, client);
    m_layoutDirty = false;
    m_fullPaintDirty = false;
    BitBlt(hdc, 0, 0, w, h, m_paintMemDc, 0, 0, SRCCOPY);
  } else {
    PaintFull(m_paintMemDc, client, &paintRect);
    BitBlt(hdc, paintRect.left, paintRect.top,
           paintRect.right - paintRect.left, paintRect.bottom - paintRect.top,
           m_paintMemDc, paintRect.left, paintRect.top, SRCCOPY);
  }
  const unsigned long long fullPct =
      m_paintCount == 0 ? 0 : (m_fullPaintCount * 100 / m_paintCount);
  wchar_t line[224] = {};
  swprintf_s(line,
             L"stage=CandidateWindow/Paint elapsed_ms=%llu full=%u paint_count=%llu full_count=%llu full_pct=%llu",
             static_cast<unsigned long long>(GetTickCount64() - paintStart),
             fullPaint ? 1u : 0u, m_paintCount, m_fullPaintCount, fullPct);
  SrfTsfPerfLog(L"perf", line);
}

int CCandidateWindow::HitTest(POINT pt) const {
  for (size_t i = 0; i < m_itemRects.size(); ++i) {
    if (PtInRect(&m_itemRects[i], pt)) return static_cast<int>(i);
  }
  return -1;
}

int CCandidateWindow::HitTestPinMenuRaw(POINT pt) const {
  if (!m_pinMenuVisible) return kPinMenuCommandNone;
  if (PtInRect(&m_pinMenuPinRect, pt)) return kPinMenuCommandPin;
  if (PtInRect(&m_pinMenuUnpinRect, pt)) return kPinMenuCommandUnpin;
  if (PtInRect(&m_pinMenuRemoveRect, pt)) return kPinMenuCommandRemove;
  if (PtInRect(&m_pinMenuBlockRect, pt)) return kPinMenuCommandBlock;
  if (PtInRect(&m_pinMenuSourceRect, pt)) return kPinMenuCommandSource;
  return kPinMenuCommandNone;
}

int CCandidateWindow::HitTestPinMenu(POINT pt) const {
  const int command = HitTestPinMenuRaw(pt);
  return PinMenuCommandEnabled(command) ? command : kPinMenuCommandNone;
}

bool CCandidateWindow::PinMenuCommandEnabled(int command) const {
  switch (command) {
    case kPinMenuCommandPin:
      return !m_pinMenuItemPinned;
    case kPinMenuCommandUnpin:
      return m_pinMenuItemPinned;
    case kPinMenuCommandRemove:
    case kPinMenuCommandBlock:
    case kPinMenuCommandSource:
      return true;
    default:
      return false;
  }
}

void CCandidateWindow::LayoutPinMenuRect(int clientWidth, int contentBottom) const {
  const LayoutSpec spec = ResolveLayoutSpec(m_style);
  const int outerPadX = Scale(spec.outerPadX);
  const int gap = Scale(6);
  const int buttonGap = Scale(4);
  const int barHeight = std::max(Scale(34), Scale(static_cast<int>(m_style.candidateFontSize) + 18));
  const int top = contentBottom + gap;
  const int menuBottom = top + barHeight * 2 + buttonGap;
  m_pinMenuRect = {outerPadX, top, clientWidth - outerPadX, menuBottom};
  const int usableLeft = m_pinMenuRect.left + buttonGap;
  const int usableRight = m_pinMenuRect.right - buttonGap;
  const int row1Top = m_pinMenuRect.top + buttonGap;
  const int row1Bottom = row1Top + barHeight;
  const int row2Top = row1Bottom + buttonGap;
  const int row2Bottom = row2Top + barHeight;
  const int row1Mid = (usableLeft + usableRight) / 2;
  const int row2W = std::max(1, (usableRight - usableLeft - buttonGap * 2) / 3);
  m_pinMenuPinRect = {usableLeft, row1Top, row1Mid - buttonGap / 2, row1Bottom};
  m_pinMenuUnpinRect = {row1Mid + buttonGap / 2, row1Top, usableRight, row1Bottom};
  m_pinMenuRemoveRect = {usableLeft, row2Top, usableLeft + row2W, row2Bottom};
  m_pinMenuBlockRect = {m_pinMenuRemoveRect.right + buttonGap, row2Top,
                        m_pinMenuRemoveRect.right + buttonGap + row2W, row2Bottom};
  m_pinMenuSourceRect = {m_pinMenuBlockRect.right + buttonGap, row2Top, usableRight, row2Bottom};
}

void CCandidateWindow::ShowPinMenu(UINT indexInPage) {
  if (!m_hwnd || !m_interactive) return;
  m_pinMenuVisible = true;
  m_pinMenuIndex = indexInPage;
  m_pinMenuItemPinned =
      indexInPage < m_pinnedItems.size() ? m_pinnedItems[indexInPage] : false;
  m_pinMenuHotCommand = kPinMenuCommandNone;
  m_pinMenuPressedCommand = kPinMenuCommandNone;
  m_layoutDirty = true;
  m_staticPaintDirty = true;
  if (m_hasAnchorRect) {
    const RECT work = PlacementAreaForAnchor(&m_anchorRect, m_fullscreenOverlayPlacement);
    const int screenInset = std::max(Scale(10), CandidateShadowMarginForStyle(m_style, m_dpi));
    const int maxWidth =
        std::max(Scale(260), static_cast<int>((work.right - work.left) - screenInset * 2));
    m_measuredClientSize = MeasureClientSize(maxWidth, &m_itemRects);
    const RECT rect = CalculateWindowRect(m_anchorRect, m_measuredClientSize);
    ApplyWindowRect(rect);
  }
  InvalidateRect(m_hwnd, nullptr, FALSE);
  std::wstring line = L"indexInPage=";
  line += std::to_wstring(indexInPage);
  line += L", menuRect=(";
  line += std::to_wstring(m_pinMenuRect.left);
  line += L",";
  line += std::to_wstring(m_pinMenuRect.top);
  line += L",";
  line += std::to_wstring(m_pinMenuRect.right);
  line += L",";
  line += std::to_wstring(m_pinMenuRect.bottom);
  line += L")";
  SrfTsfDiagnosticLog(L"candidate-pin-menu.show", line.c_str());
}

void CCandidateWindow::HidePinMenu() {
  if (!m_pinMenuVisible && m_pinMenuPressedCommand == kPinMenuCommandNone &&
      m_pinMenuHotCommand == kPinMenuCommandNone) {
    return;
  }
  SrfTsfDiagnosticLog(L"candidate-pin-menu.hide", L"hide");
  m_pinMenuVisible = false;
  m_pinMenuIndex = 0;
  m_pinMenuItemPinned = false;
  m_pinMenuHotCommand = kPinMenuCommandNone;
  m_pinMenuPressedCommand = kPinMenuCommandNone;
  m_pinMenuRect = {};
  m_pinMenuPinRect = {};
  m_pinMenuUnpinRect = {};
  m_pinMenuRemoveRect = {};
  m_pinMenuBlockRect = {};
  m_pinMenuSourceRect = {};
  m_layoutDirty = true;
  m_staticPaintDirty = true;
  if (m_hwnd && m_hasAnchorRect) {
    const RECT work = PlacementAreaForAnchor(&m_anchorRect, m_fullscreenOverlayPlacement);
    const int screenInset = std::max(Scale(10), CandidateShadowMarginForStyle(m_style, m_dpi));
    const int maxWidth =
        std::max(Scale(260), static_cast<int>((work.right - work.left) - screenInset * 2));
    m_measuredClientSize = MeasureClientSize(maxWidth, &m_itemRects);
    const RECT rect = CalculateWindowRect(m_anchorRect, m_measuredClientSize);
    ApplyWindowRect(rect);
  }
  if (m_hwnd) InvalidateRect(m_hwnd, nullptr, FALSE);
}

void CCandidateWindow::FlushPendingInvalidates() {
  if (!m_hasPendingDirtyRgn || !m_pendingDirtyRgn || !m_hwnd) return;
  InvalidateRgn(m_hwnd, m_pendingDirtyRgn, FALSE);
  DeleteObject(m_pendingDirtyRgn);
  m_pendingDirtyRgn = nullptr;
  m_hasPendingDirtyRgn = false;
}

void CCandidateWindow::ScheduleDeferredPaint(ULONGLONG now) {
  if (!m_hwnd) return;
  FlushPendingInvalidates();
  const ULONGLONG elapsed = m_lastShowTick == 0 ? kCandidateMinImmediatePaintIntervalMs
                                                : now - m_lastShowTick;
  const UINT delay = static_cast<UINT>(
      elapsed >= kCandidateMinImmediatePaintIntervalMs
          ? 1
          : std::max<ULONGLONG>(1, kCandidateMinImmediatePaintIntervalMs - elapsed));
  if (SetTimer(m_hwnd, kCandidatePaintTimerId, delay, nullptr)) {
    m_paintTimerPending = true;
  } else {
    FlushDeferredPaint();
  }
}

void CCandidateWindow::FlushDeferredPaint() {
  FlushPendingPaint();
}

void CCandidateWindow::UpdatePendingIndicatorTimer(bool pendingVisual) {
  if (!pendingVisual) {
    if (m_hwnd && m_pendingIndicatorTimerPending) {
      KillTimer(m_hwnd, kCandidatePendingIndicatorTimerId);
    }
    m_pendingIndicatorTimerPending = false;
    m_pendingVisualSince = 0;
    return;
  }

  if (!m_pendingVisual) {
    m_pendingVisualSince = GetTickCount64();
  }
  if (!m_hwnd || m_pendingIndicatorTimerPending) return;
  const ULONGLONG now = GetTickCount64();
  if (m_pendingVisualSince != 0 && now >= m_pendingVisualSince &&
      now - m_pendingVisualSince >= kCandidatePendingIndicatorDelayMs) {
    return;
  }
  if (SetTimer(m_hwnd, kCandidatePendingIndicatorTimerId,
               kCandidatePendingIndicatorDelayMs, nullptr)) {
    m_pendingIndicatorTimerPending = true;
  }
}

void CCandidateWindow::ScheduleEnvironmentRefresh() {
  if (!m_hwnd || !IsWindowVisible(m_hwnd)) return;
  if (m_environmentRefreshTimerPending) {
    KillTimer(m_hwnd, kCandidateEnvironmentRefreshTimerId);
    m_environmentRefreshTimerPending = false;
  }
  m_environmentRefreshForced = true;
  if (SetTimer(m_hwnd, kCandidateEnvironmentRefreshTimerId,
               kCandidateEnvironmentRefreshDelayMs, nullptr)) {
    m_environmentRefreshTimerPending = true;
    return;
  }
  m_environmentRefreshForced = false;
  if (m_events) m_events->OnCandidateEnvironmentChanged();
}

void CCandidateWindow::ScheduleOverlayEnvironmentPoll() {
  if (!m_gameOverlay || !m_hwnd || !IsWindowVisible(m_hwnd) ||
      m_environmentRefreshTimerPending) {
    return;
  }
  if (!m_overlayEnvironmentValid) CaptureOverlayEnvironment();
  m_environmentRefreshForced = false;
  if (SetTimer(m_hwnd, kCandidateEnvironmentRefreshTimerId, kCandidateEnvironmentPollMs,
               nullptr)) {
    m_environmentRefreshTimerPending = true;
  }
}

void CCandidateWindow::CancelEnvironmentRefresh() {
  if (m_hwnd && m_environmentRefreshTimerPending) {
    KillTimer(m_hwnd, kCandidateEnvironmentRefreshTimerId);
  }
  m_environmentRefreshTimerPending = false;
  m_environmentRefreshForced = false;
}

void CCandidateWindow::FlushEnvironmentRefresh() {
  if (!m_environmentRefreshTimerPending) return;
  const bool forced = m_environmentRefreshForced;
  CancelEnvironmentRefresh();
  const bool changed = forced || OverlayEnvironmentChanged();
  if (changed && m_events && m_hwnd && IsWindowVisible(m_hwnd)) {
    m_events->OnCandidateEnvironmentChanged();
  }
  CaptureOverlayEnvironment();
  ScheduleOverlayEnvironmentPoll();
}

bool CCandidateWindow::OverlayEnvironmentChanged() {
  if (!m_gameOverlay) return false;
  if (!m_overlayTargetHwnd || !IsWindow(m_overlayTargetHwnd)) return true;

  RECT targetRect = {};
  if (!GetWindowRect(m_overlayTargetHwnd, &targetRect)) return true;
  const HMONITOR monitor = MonitorFromWindow(m_overlayTargetHwnd, MONITOR_DEFAULTTONEAREST);
  const UINT dpi = DpiForScreenRect(&targetRect);
  return !m_overlayEnvironmentValid || monitor != m_overlayObservedMonitor ||
         !EqualRect(&targetRect, &m_overlayObservedTargetRect) || dpi != m_overlayObservedDpi;
}

void CCandidateWindow::CaptureOverlayEnvironment() {
  m_overlayEnvironmentValid = false;
  m_overlayObservedMonitor = nullptr;
  m_overlayObservedTargetRect = {};
  m_overlayObservedDpi = 0;
  if (!m_gameOverlay || !m_overlayTargetHwnd || !IsWindow(m_overlayTargetHwnd)) return;

  RECT targetRect = {};
  if (!GetWindowRect(m_overlayTargetHwnd, &targetRect)) return;
  m_overlayObservedMonitor =
      MonitorFromWindow(m_overlayTargetHwnd, MONITOR_DEFAULTTONEAREST);
  m_overlayObservedTargetRect = targetRect;
  m_overlayObservedDpi = DpiForScreenRect(&targetRect);
  m_overlayEnvironmentValid = m_overlayObservedMonitor != nullptr;
}

void CCandidateWindow::CancelPendingHorizontalShrink() {
  if (m_hwnd && m_horizontalShrinkTimerPending) {
    KillTimer(m_hwnd, kCandidateHorizontalShrinkTimerId);
  }
  m_horizontalShrinkTimerPending = false;
  m_pendingHorizontalShrinkSize = {};
  m_pendingHorizontalShrinkAnchor = {};
  m_pendingHorizontalShrinkItemRects.clear();
}

void CCandidateWindow::SchedulePendingHorizontalShrink(const RECT& anchorRect, SIZE clientSize,
                                                       const std::vector<RECT>& itemRects) {
  if (!m_hwnd || clientSize.cx <= 0 || clientSize.cy <= 0) return;
  m_pendingHorizontalShrinkSize = clientSize;
  m_pendingHorizontalShrinkAnchor = anchorRect;
  m_pendingHorizontalShrinkItemRects = itemRects;
  if (SetTimer(m_hwnd, kCandidateHorizontalShrinkTimerId, kCandidateHorizontalShrinkDelayMs, nullptr)) {
    m_horizontalShrinkTimerPending = true;
  } else {
    FlushPendingHorizontalShrink();
  }
}

void CCandidateWindow::FlushPendingHorizontalShrink() {
  if (!m_hwnd) return;
  if (m_horizontalShrinkTimerPending) {
    KillTimer(m_hwnd, kCandidateHorizontalShrinkTimerId);
    m_horizontalShrinkTimerPending = false;
  }
  const SIZE targetSize = m_pendingHorizontalShrinkSize;
  const RECT targetAnchor = m_pendingHorizontalShrinkAnchor;
  std::vector<RECT> targetItemRects = std::move(m_pendingHorizontalShrinkItemRects);
  m_pendingHorizontalShrinkSize = {};
  m_pendingHorizontalShrinkAnchor = {};
  m_pendingHorizontalShrinkItemRects.clear();

  if (!IsWindowVisible(m_hwnd) || !m_style.candidateHorizontal || !m_hasAnchorRect ||
      targetSize.cx <= 0 || targetSize.cy <= 0 || targetSize.cx >= m_measuredClientSize.cx ||
      !EqualRect(&m_anchorRect, &targetAnchor)) {
    return;
  }

  m_measuredClientSize = targetSize;
  if (!targetItemRects.empty()) {
    m_itemRects = std::move(targetItemRects);
  }
  m_needsMeasure = false;
  m_layoutDirty = true;
  m_staticPaintDirty = true;
  const RECT rect = CalculateWindowRect(m_anchorRect, m_measuredClientSize);
  ApplyWindowRect(rect);
  InvalidateRect(m_hwnd, nullptr, FALSE);
  UpdateWindow(m_hwnd);
  m_lastShowTick = GetTickCount64();
}

void CCandidateWindow::InvalidateCandidateIndex(int index) {
  if (!m_hwnd || index < 0 || static_cast<size_t>(index) >= m_itemRects.size()) return;
  RECT dirty = m_itemRects[static_cast<size_t>(index)];
  if (!m_style.candidateHorizontal &&
      m_style.candidateLayoutVariant != SrfCandidateLayoutVariant::Card) {
    RECT client = {};
    GetClientRect(m_hwnd, &client);
    dirty.left = client.left;
    dirty.right = client.right;
    if (index > 0) {
      const RECT& previous = m_itemRects[static_cast<size_t>(index - 1)];
      dirty.top = (previous.bottom + dirty.top) / 2 - Scale(2);
    } else {
      dirty.top -= Scale(2);
    }
    if (static_cast<size_t>(index + 1) < m_itemRects.size()) {
      const RECT& next = m_itemRects[static_cast<size_t>(index + 1)];
      dirty.bottom = (dirty.bottom + next.top) / 2 + Scale(2);
    } else {
      dirty.bottom += Scale(2);
    }
    RECT clipped = {};
    if (!IntersectRect(&clipped, &dirty, &client)) return;
    dirty = clipped;
  } else {
    InflateRect(&dirty, Scale(2), Scale(2));
  }

  HRGN rgn = CreateRectRgn(dirty.left, dirty.top, dirty.right, dirty.bottom);
  if (!rgn) {
    InvalidateRect(m_hwnd, &dirty, FALSE);
    return;
  }
  if (m_hasPendingDirtyRgn && m_pendingDirtyRgn) {
    CombineRgn(m_pendingDirtyRgn, m_pendingDirtyRgn, rgn, RGN_OR);
    DeleteObject(rgn);
  } else {
    m_pendingDirtyRgn = rgn;
    m_hasPendingDirtyRgn = true;
  }
}

void CCandidateWindow::UpdateHotIndex(int hotIndex) {
  if (m_hotIndex == hotIndex) return;
  const int previous = m_hotIndex;
  m_hotIndex = hotIndex;
  StartHoverAnimation(previous, m_hotIndex);
  InvalidateCandidateIndex(previous);
  InvalidateCandidateIndex(m_hotIndex);
}

void CCandidateWindow::UpdatePressedIndex(int pressedIndex) {
  if (m_pressedIndex == pressedIndex) return;
  const int previous = m_pressedIndex;
  m_pressedIndex = pressedIndex;
  StartPressAnimation(previous, m_pressedIndex);
  InvalidateCandidateIndex(previous);
  InvalidateCandidateIndex(m_pressedIndex);
}

void CCandidateWindow::UpdatePinMenuHotCommand(int command) {
  if (m_pinMenuHotCommand == command) return;
  m_pinMenuHotCommand = command;
  if (m_hwnd && m_pinMenuVisible) {
    InvalidateRect(m_hwnd, &m_pinMenuRect, FALSE);
  }
}

void CCandidateWindow::BeginTrackMouseLeave() {
  if (!m_hwnd || m_trackingMouse) return;
  TRACKMOUSEEVENT tme = {};
  tme.cbSize = sizeof(tme);
  tme.dwFlags = TME_LEAVE;
  tme.hwndTrack = m_hwnd;
  if (TrackMouseEvent(&tme)) m_trackingMouse = true;
}

bool CCandidateWindow::MotionEnabled() const {
  BOOL clientAnimations = TRUE;
  if (!SystemParametersInfoW(SPI_GETCLIENTAREAANIMATION, 0, &clientAnimations, 0)) {
    clientAnimations = TRUE;
  }
  HIGHCONTRASTW highContrast = {};
  highContrast.cbSize = sizeof(highContrast);
  const bool systemHighContrast =
      SystemParametersInfoW(SPI_GETHIGHCONTRAST, sizeof(highContrast), &highContrast, 0) &&
      (highContrast.dwFlags & HCF_HIGHCONTRASTON) != 0;
  return ShouldAnimateCandidateWindow(
      m_style.candidateReduceMotion, clientAnimations != FALSE,
      m_style.themeMode == SrfThemeMode::HighContrast || systemHighContrast, m_gameOverlay,
      !m_style.skinLoaded || m_style.skinAnimationsEnabled);
}

int CCandidateWindow::ResolveAnimationDuration(int skinDuration, int fallbackMs) const {
  return std::clamp(skinDuration >= 0 ? skinDuration : fallbackMs, 0, 240);
}

void CCandidateWindow::StartShowAnimation() {
  const int duration =
      ResolveAnimationDuration(m_style.skinShowAnimationMs, kCandidateShowAnimationMs);
  if (!MotionEnabled() || duration <= 0 || !m_hasTargetWindowRect) {
    m_showAnimationActive = false;
    ApplyAnimatedWindowRect(1.0f);
    ApplyWindowOpacity();
    UpdateShadowWindow();
    return;
  }
  m_showAnimationActive = true;
  m_showAnimationStart = GetTickCount64();
  m_showAnimationDurationMs = duration;
  ApplyAnimatedWindowRect(0.0f);
  ApplyWindowOpacity();
  UpdateShadowWindow();
  ScheduleAnimationFrame();
}

void CCandidateWindow::StartSelectionAnimation(int previousIndex, int nextIndex) {
  const int duration =
      ResolveAnimationDuration(m_style.skinSelectionAnimationMs, kCandidateSelectionAnimationMs);
  if (!MotionEnabled() || duration <= 0 || previousIndex < 0 || nextIndex < 0 ||
      previousIndex == nextIndex || static_cast<size_t>(previousIndex) >= m_itemRects.size() ||
      static_cast<size_t>(nextIndex) >= m_itemRects.size()) {
    m_selectionAnimationActive = false;
    return;
  }
  m_selectionAnimationActive = true;
  m_selectionAnimationFrom = previousIndex;
  m_selectionAnimationTo = nextIndex;
  m_selectionAnimationStart = GetTickCount64();
  m_selectionAnimationDurationMs = duration;
  ScheduleAnimationFrame();
}

void CCandidateWindow::StartHoverAnimation(int previousIndex, int nextIndex) {
  const int duration =
      ResolveAnimationDuration(m_style.skinHoverAnimationMs, kCandidateHoverAnimationMs);
  if (!MotionEnabled() || duration <= 0 || previousIndex == nextIndex) {
    m_hoverAnimationActive = false;
    return;
  }
  m_hoverAnimationActive = true;
  m_hoverAnimationFrom = previousIndex;
  m_hoverAnimationTo = nextIndex;
  m_hoverAnimationStart = GetTickCount64();
  m_hoverAnimationDurationMs = duration;
  ScheduleAnimationFrame();
}

void CCandidateWindow::StartPressAnimation(int previousIndex, int nextIndex) {
  const int duration =
      ResolveAnimationDuration(m_style.skinPressAnimationMs, kCandidatePressAnimationMs);
  if (!MotionEnabled() || duration <= 0 || previousIndex == nextIndex) {
    m_pressAnimationActive = false;
    return;
  }
  m_pressAnimationActive = true;
  m_pressAnimationFrom = previousIndex;
  m_pressAnimationTo = nextIndex;
  m_pressAnimationStart = GetTickCount64();
  m_pressAnimationDurationMs = duration;
  ScheduleAnimationFrame();
}

void CCandidateWindow::StartPageAnimation(int previousPage, int nextPage) {
  const int duration =
      ResolveAnimationDuration(m_style.skinPageAnimationMs, kCandidatePageAnimationMs);
  if (!MotionEnabled() || duration <= 0 || previousPage == nextPage) {
    m_pageAnimationActive = false;
    return;
  }
  m_selectionAnimationActive = false;
  m_pageAnimationActive = true;
  m_pageAnimationDirection = nextPage > previousPage ? 1 : -1;
  m_pageAnimationStart = GetTickCount64();
  m_pageAnimationDurationMs = duration;
  ScheduleAnimationFrame();
}

void CCandidateWindow::CancelAnimations(bool restoreWindowState) {
  if (m_hwnd && m_animationTimerPending) {
    KillTimer(m_hwnd, kCandidateAnimationTimerId);
  }
  m_animationTimerPending = false;
  m_showAnimationActive = false;
  m_selectionAnimationActive = false;
  m_hoverAnimationActive = false;
  m_pressAnimationActive = false;
  m_pageAnimationActive = false;
  if (restoreWindowState && m_hwnd) {
    ApplyAnimatedWindowRect(1.0f);
    ApplyWindowOpacity();
    UpdateShadowWindow();
  }
}

void CCandidateWindow::ScheduleAnimationFrame() {
  if (!m_hwnd || m_animationTimerPending) return;
  if (SetTimer(m_hwnd, kCandidateAnimationTimerId, kCandidateAnimationFrameMs, nullptr)) {
    m_animationTimerPending = true;
  }
}

void CCandidateWindow::AdvanceAnimations() {
  if (!m_hwnd) return;
  if (m_animationTimerPending) {
    KillTimer(m_hwnd, kCandidateAnimationTimerId);
    m_animationTimerPending = false;
  }
  const ULONGLONG now = GetTickCount64();
  bool visualAnimation = false;
  if (m_showAnimationActive) {
    const float progress =
        LinearAnimationProgress(now, m_showAnimationStart, m_showAnimationDurationMs);
    ApplyAnimatedWindowRect(EaseOutCubic(progress));
    ApplyWindowOpacity();
    UpdateShadowWindow();
    if (progress >= 1.0f) {
      m_showAnimationActive = false;
      ApplyAnimatedWindowRect(1.0f);
      ApplyWindowOpacity();
      UpdateShadowWindow();
    }
  }
  auto advanceIndexAnimation = [now, &visualAnimation](bool& active, ULONGLONG start,
                                                       int duration) {
    if (!active) return;
    visualAnimation = true;
    if (LinearAnimationProgress(now, start, duration) >= 1.0f) active = false;
  };
  advanceIndexAnimation(m_selectionAnimationActive, m_selectionAnimationStart,
                        m_selectionAnimationDurationMs);
  advanceIndexAnimation(m_hoverAnimationActive, m_hoverAnimationStart, m_hoverAnimationDurationMs);
  advanceIndexAnimation(m_pressAnimationActive, m_pressAnimationStart, m_pressAnimationDurationMs);
  advanceIndexAnimation(m_pageAnimationActive, m_pageAnimationStart, m_pageAnimationDurationMs);
  if (visualAnimation) {
    InvalidateRect(m_hwnd, nullptr, FALSE);
    UpdateWindow(m_hwnd);
  }
  if (m_showAnimationActive || m_selectionAnimationActive || m_hoverAnimationActive ||
      m_pressAnimationActive || m_pageAnimationActive) {
    ScheduleAnimationFrame();
  }
}

ATOM CCandidateWindow::EnsureWindowClass() {
  static ATOM atom = 0;
  if (atom != 0) return atom;

  WNDCLASSW wc = {};
  wc.lpfnWndProc = &CCandidateWindow::WndProc;
  wc.hInstance = DllOrFallbackInstance();
  wc.lpszClassName = kCandidateWndClass;
  wc.hCursor = LoadCursorW(nullptr, IDC_ARROW);
  atom = RegisterClassW(&wc);
  if (!atom && GetLastError() == ERROR_CLASS_ALREADY_EXISTS) atom = 1;
  return atom;
}

LRESULT CALLBACK CCandidateWindow::WndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
  CCandidateWindow* self =
      reinterpret_cast<CCandidateWindow*>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));

  if (msg == WM_NCCREATE) {
    CREATESTRUCTW* cs = reinterpret_cast<CREATESTRUCTW*>(lParam);
    self = reinterpret_cast<CCandidateWindow*>(cs->lpCreateParams);
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
    if (self) self->m_hwnd = hwnd;
    return TRUE;
  }

  if (!self) return DefWindowProcW(hwnd, msg, wParam, lParam);

  switch (msg) {
    case WM_NCHITTEST:
      if (self->m_gameOverlay) return HTTRANSPARENT;
      break;
    case WM_MOUSEACTIVATE:
      return MA_NOACTIVATE;
    case WM_ERASEBKGND:
      return 1;
    case WM_TIMER:
      if (wParam == kCandidateAnimationTimerId) {
        self->AdvanceAnimations();
        return 0;
      }
      if (wParam == kCandidatePaintTimerId) {
        self->FlushDeferredPaint();
        return 0;
      }
      if (wParam == kCandidatePendingIndicatorTimerId) {
        self->m_pendingIndicatorTimerPending = false;
        KillTimer(hwnd, kCandidatePendingIndicatorTimerId);
        if (self->m_pendingVisual) {
          self->m_staticPaintDirty = true;
          InvalidateRect(hwnd, nullptr, FALSE);
        }
        return 0;
      }
      if (wParam == kCandidateHorizontalShrinkTimerId) {
        self->FlushPendingHorizontalShrink();
        return 0;
      }
      if (wParam == kCandidateEnvironmentRefreshTimerId) {
        self->FlushEnvironmentRefresh();
        return 0;
      }
      break;
    case WM_DISPLAYCHANGE:
      self->ScheduleEnvironmentRefresh();
      SrfTsfDiagnosticLog(L"candidate-window.environment", L"display-change");
      return 0;
    case WM_SETTINGCHANGE:
      if (wParam == SPI_SETWORKAREA) {
        self->ScheduleEnvironmentRefresh();
        SrfTsfDiagnosticLog(L"candidate-window.environment", L"work-area-change");
        return 0;
      }
      break;
    case WM_DWMCOMPOSITIONCHANGED:
      self->ScheduleEnvironmentRefresh();
      return 0;
    case WM_DPICHANGED: {
      self->CancelPendingHorizontalShrink();
      self->m_dpi = HIWORD(wParam) != 0 ? HIWORD(wParam) : LOWORD(wParam);
      if (self->m_dpi == 0) self->m_dpi = 96;
      self->m_layoutDirty = true;
      self->m_needsMeasure = true;
      self->m_measuredClientSize = {};
      self->RefreshFonts();
      self->m_itemRects.clear();
      self->m_pinMenuRect = {};
      self->m_pinMenuPinRect = {};
      self->m_pinMenuUnpinRect = {};
      self->m_pinMenuRemoveRect = {};
      self->m_pinMenuBlockRect = {};
      self->m_pinMenuSourceRect = {};
      self->m_pinMenuVisible = false;
      self->m_pinMenuIndex = 0;
      self->m_pinMenuItemPinned = false;
      self->m_pinMenuHotCommand = kPinMenuCommandNone;
      self->m_pinMenuPressedCommand = kPinMenuCommandNone;
      self->ReleasePaintBuffer();
      self->ReleaseStaticPaintBuffer();
      self->ReleaseItemPaintCaches();
      self->ClearMeasuredTextCaches();
      GetGdiCache().Reset();
      RECT* suggested = reinterpret_cast<RECT*>(lParam);
      if (suggested) {
        self->ApplyWindowRect(*suggested);
      }
      InvalidateRect(hwnd, nullptr, FALSE);
      self->ScheduleEnvironmentRefresh();
      return 0;
    }
    case WM_SETCURSOR: {
      if (!self->m_interactive && LOWORD(lParam) == HTCLIENT) {
        SetCursor(LoadCursorW(nullptr, IDC_ARROW));
        return TRUE;
      }
      if (LOWORD(lParam) == HTCLIENT) {
        POINT pt = {};
        if (GetCursorPos(&pt) && ScreenToClient(hwnd, &pt)) {
          const bool overMenuCommand =
              self->m_pinMenuVisible && self->HitTestPinMenu(pt) != kPinMenuCommandNone;
          const bool overCandidate =
              !self->m_pinMenuVisible &&
              (self->m_style.candidateLeftClick || self->m_style.candidateRightClick) &&
              self->HitTest(pt) >= 0;
          SetCursor(LoadCursorW(nullptr, overMenuCommand || overCandidate ? IDC_HAND : IDC_ARROW));
          return TRUE;
        }
      }
      break;
    }
    case WM_MOUSEMOVE: {
      if (!self->m_interactive) {
        self->UpdatePinMenuHotCommand(kPinMenuCommandNone);
        self->UpdateHotIndex(-1);
        return 0;
      }
      self->BeginTrackMouseLeave();
      POINT pt = {GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam)};
      if (self->m_pinMenuVisible) {
        const int rawPinCommand = self->HitTestPinMenuRaw(pt);
        self->UpdatePinMenuHotCommand(
            self->PinMenuCommandEnabled(rawPinCommand) ? rawPinCommand : kPinMenuCommandNone);
        self->UpdateHotIndex(-1);
      } else {
        self->UpdatePinMenuHotCommand(kPinMenuCommandNone);
        self->UpdateHotIndex(self->HitTest(pt));
      }
      return 0;
    }
    case WM_MOUSELEAVE:
      self->m_trackingMouse = false;
      self->UpdatePinMenuHotCommand(kPinMenuCommandNone);
      self->UpdateHotIndex(-1);
      return 0;
    case WM_LBUTTONDOWN: {
      if (!self->m_interactive) return 0;
      self->m_rightPressedIndex = -1;
      POINT pt = {GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam)};
      if (self->m_pinMenuVisible) {
        const int rawPinCommand = self->HitTestPinMenuRaw(pt);
        const int pinCommand =
            self->PinMenuCommandEnabled(rawPinCommand) ? rawPinCommand : kPinMenuCommandNone;
        if (pinCommand != kPinMenuCommandNone) {
          SetCapture(hwnd);
          self->UpdatePinMenuHotCommand(pinCommand);
          self->m_pinMenuPressedCommand = pinCommand;
          InvalidateRect(hwnd, &self->m_pinMenuRect, FALSE);
        } else if (rawPinCommand == kPinMenuCommandNone) {
          self->HidePinMenu();
        } else {
          InvalidateRect(hwnd, &self->m_pinMenuRect, FALSE);
        }
        return 0;
      }
      if (!self->m_style.candidateLeftClick) return 0;
      SetCapture(hwnd);
      self->UpdatePressedIndex(self->HitTest(pt));
      return 0;
    }
    case WM_LBUTTONUP: {
      if (!self->m_interactive) {
        if (GetCapture() == hwnd) ReleaseCapture();
        self->UpdatePressedIndex(-1);
        return 0;
      }
      const int pressed = self->m_pressedIndex;
      const int pressedCommand = self->m_pinMenuPressedCommand;
      POINT pt = {GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam)};
      if (GetCapture() == hwnd) ReleaseCapture();
      if (self->m_pinMenuVisible || self->m_pinMenuPressedCommand != kPinMenuCommandNone) {
        const int pinCommand = self->HitTestPinMenu(pt);
        self->m_pinMenuPressedCommand = kPinMenuCommandNone;
        if (pinCommand != kPinMenuCommandNone && pinCommand == pressedCommand && self->m_events) {
          const UINT indexInPage = self->m_pinMenuIndex;
          self->HidePinMenu();
          if (pinCommand == kPinMenuCommandPin || pinCommand == kPinMenuCommandUnpin) {
            self->m_events->OnCandidatePinRequested(indexInPage, pinCommand == kPinMenuCommandPin);
          } else {
            self->m_events->OnCandidateMenuCommand(indexInPage, pinCommand);
          }
        } else {
          InvalidateRect(hwnd, &self->m_pinMenuRect, FALSE);
        }
        return 0;
      }
      if (!self->m_style.candidateLeftClick) return 0;
      const int hit = self->HitTest(pt);
      self->UpdatePressedIndex(-1);
      if (hit >= 0 && hit == pressed && self->m_events) {
        self->m_events->OnCandidateClicked(static_cast<UINT>(hit));
      }
      return 0;
    }
    case WM_RBUTTONUP: {
      if (!self->m_interactive) {
        if (GetCapture() == hwnd) ReleaseCapture();
        self->m_rightPressedIndex = -1;
        self->UpdatePressedIndex(-1);
        return 0;
      }
      if (!self->m_style.candidateRightClick) return 0;
      const int pressed = self->m_rightPressedIndex;
      POINT pt = {GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam)};
      if (GetCapture() == hwnd) ReleaseCapture();
      const int hit = self->HitTest(pt);
      self->m_rightPressedIndex = -1;
      self->UpdatePressedIndex(-1);
      if (hit >= 0 && hit == pressed && self->m_events) {
        std::wstring line = L"hit=";
        line += std::to_wstring(hit);
        line += L", pt=(";
        line += std::to_wstring(pt.x);
        line += L",";
        line += std::to_wstring(pt.y);
        line += L")";
        SrfTsfDiagnosticLog(L"candidate-pin-menu.right-click", line.c_str());
        POINT screenPt = pt;
        ClientToScreen(hwnd, &screenPt);
        self->m_events->OnCandidateRightClicked(static_cast<UINT>(hit), screenPt);
        self->ShowPinMenu(static_cast<UINT>(hit));
      } else {
        SrfTsfDiagnosticLog(L"candidate-pin-menu.right-click-miss", L"no candidate hit");
      }
      return 0;
    }
    case WM_RBUTTONDOWN: {
      if (!self->m_interactive) return 0;
      if (!self->m_style.candidateRightClick) return 0;
      if (self->m_pinMenuVisible) {
        self->HidePinMenu();
      }
      POINT pt = {GET_X_LPARAM(lParam), GET_Y_LPARAM(lParam)};
      const int hit = self->HitTest(pt);
      self->m_rightPressedIndex = hit;
      self->UpdatePressedIndex(hit);
      if (hit >= 0) {
        SetCapture(hwnd);
      }
      return 0;
    }
    case WM_CAPTURECHANGED:
      self->UpdatePressedIndex(-1);
      self->UpdatePinMenuHotCommand(kPinMenuCommandNone);
      self->m_pinMenuPressedCommand = kPinMenuCommandNone;
      self->m_rightPressedIndex = -1;
      return 0;
    case WM_MOUSEWHEEL:
      if (!self->m_interactive) return 0;
      if (self->m_events) self->m_events->OnCandidateWheel(GET_WHEEL_DELTA_WPARAM(wParam));
      return 0;
    case WM_PAINT: {
      PAINTSTRUCT ps = {};
      HDC hdc = BeginPaint(hwnd, &ps);
      if (hdc) {
        self->Paint(hdc, ps.rcPaint);
        EndPaint(hwnd, &ps);
      }
      return 0;
    }
    default:
      return DefWindowProcW(hwnd, msg, wParam, lParam);
  }
  return DefWindowProcW(hwnd, msg, wParam, lParam);
}
