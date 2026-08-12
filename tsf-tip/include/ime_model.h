#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <msctf.h>

#include <algorithm>
#include <string>
#include <unordered_map>
#include <vector>

enum class SrfTextAttributeType { None = 0, Highlighted };

struct SrfTextRange {
  size_t start = 0;
  size_t end = 0;
  int cursor = -1;
};

struct SrfTextAttribute {
  SrfTextRange range = {};
  SrfTextAttributeType type = SrfTextAttributeType::None;
};

struct SrfText {
  std::wstring str;
  std::vector<SrfTextAttribute> attributes;

  void Clear() {
    str.clear();
    attributes.clear();
  }

  bool Empty() const { return str.empty(); }
};

struct SrfCandidateInfo {
  UINT currentPage = 0;
  UINT totalPages = 0;
  UINT highlighted = 0;
  bool isLastPage = false;
  std::vector<SrfText> items;
  std::vector<SrfText> comments;
  std::vector<SrfText> labels;

  void Clear() {
    currentPage = 0;
    totalPages = 0;
    highlighted = 0;
    isLastPage = false;
    items.clear();
    comments.clear();
    labels.clear();
  }

  bool Empty() const { return items.empty(); }
};

struct SrfContext {
  SrfText preedit;
  SrfText aux;
  SrfCandidateInfo candidates;

  void Clear() {
    preedit.Clear();
    aux.Clear();
    candidates.Clear();
  }

  bool Empty() const { return preedit.Empty() && aux.Empty() && candidates.Empty(); }
};

struct SrfStatus {
  bool asciiMode = false;
  bool composing = false;
  bool disabled = false;
  bool fullShape = false;
  bool chinesePunctuation = true;
  bool fuzzyPinyin = false;
  bool doublePinyin = false;
  std::wstring appName;
  std::wstring modeSource;
  std::wstring notification;

  void Reset() {
    asciiMode = false;
    composing = false;
    disabled = false;
    fullShape = false;
    chinesePunctuation = true;
    fuzzyPinyin = false;
    doublePinyin = false;
    appName.clear();
    modeSource.clear();
    notification.clear();
  }
};

enum class SrfThemeMode : UINT { Auto = 0, Light = 1, Dark = 2, HighContrast = 3 };
enum class SrfCandidateMaterial : UINT { Auto = 0, Solid = 1, Gradient = 2, Mist = 3 };
enum class SrfCandidateDensity : UINT { Compact = 0, Standard = 1, Comfortable = 2 };
enum class SrfCandidateLayoutVariant : UINT { Classic = 0, Compact = 1, Card = 2 };
enum class SrfFullscreenPolicy : UINT { Off = 0, HideUi = 1, Ascii = 2, ShowUi = 3 };
enum class SrfFocusPolicy : UINT { Normal = 0, Strict = 1, Window = 2 };
enum class SrfOverlayAnchor : UINT {
  Auto = 0,
  Caret = 1,
  TopLeft = 2,
  TopCenter = 3,
  TopRight = 4,
  BottomLeft = 5,
  BottomCenter = 6,
  BottomRight = 7,
};
enum class SrfOverlayBackend : UINT { Auto = 0, InProcess = 1, External = 2 };

inline bool ShouldUseExternalCandidateOverlayBackend(
    SrfOverlayBackend backend, bool fullscreen, bool uiLess,
    bool hideCandidateUi) {
  if (hideCandidateUi) return false;
  switch (backend) {
    case SrfOverlayBackend::External:
      return true;
    case SrfOverlayBackend::InProcess:
      return false;
    case SrfOverlayBackend::Auto:
    default:
      return fullscreen || uiLess;
  }
}

enum class SrfCommitTransport : UINT {
  Tsf = 0,
  ClipboardPaste = 1,
  UnicodeSendInput = 2,
  Auto = 3
};

struct SrfUIStyle {
  bool inlinePreedit = true;
  bool enhancedPosition = true;
  bool pagingOnScroll = true;
  UINT candidateAbbreviateLength = 64;
  UINT candidateFontSize = 16;
  UINT candidateOpacity = 100;
  bool candidateReduceMotion = false;
  std::wstring candidateFontFile = L"Microsoft YaHei";
  int candidateFontWeight = 500;          // Medium
  int candidateSelectedFontWeight = 600;  // Semibold
  int candidateLabelFontWeight = 600;     // Semibold
  int candidateChipFontWeight = 500;      // Medium
  std::wstring candidateSkinFile;
  bool candidateHorizontal = true;
  UINT candidatePageSize = 9;
  /// 横向候选单行最多显示多少个候选（设置页可选 3..=9；极端窄屏/超长词仍允许布局退化到更少）。
  UINT candidateHorizontalCount = 5;
  /// 横向候选是否启用“更紧凑”的间距/内边距（字体大小不变）。
  bool candidateHorizontalCompact = false;
  bool showCandidateReading = false;
  bool showCandidateScore = false;
  bool highlightTypoCandidates = true;
  bool showCandidateSource = false;
  bool showModeInCandidateHeader = false;
  bool candidateTopmost = true;
  bool candidateLeftClick = true;
  bool candidateRightClick = true;
  SrfThemeMode themeMode = SrfThemeMode::Auto;
  SrfCandidateMaterial candidateMaterial = SrfCandidateMaterial::Auto;
  SrfCandidateDensity candidateDensity = SrfCandidateDensity::Standard;
  SrfCandidateLayoutVariant candidateLayoutVariant = SrfCandidateLayoutVariant::Classic;
  /// Per-application HUD scale, independent from monitor DPI.
  UINT candidateScalePercent = 100;
  /// Used only by fixed game-overlay anchors to align center/right positions.
  SrfOverlayAnchor candidateOverlayAnchor = SrfOverlayAnchor::Auto;
  bool candidateFullscreenPlacement = false;

  // Skin overrides (non-empty when a skin file is loaded).
  COLORREF skinWindowBg = CLR_INVALID;
  COLORREF skinWindowBgTo = CLR_INVALID;
  COLORREF skinHeaderBg = CLR_INVALID;
  COLORREF skinHeaderBgTo = CLR_INVALID;
  COLORREF skinBorder = CLR_INVALID;
  COLORREF skinDivider = CLR_INVALID;
  COLORREF skinText = CLR_INVALID;
  COLORREF skinMutedText = CLR_INVALID;
  COLORREF skinBadgeBg = CLR_INVALID;
  COLORREF skinBadgeBorder = CLR_INVALID;
  COLORREF skinBadgeText = CLR_INVALID;
  COLORREF skinHoverBg = CLR_INVALID;
  COLORREF skinHoverBorder = CLR_INVALID;
  COLORREF skinItemBg = CLR_INVALID;
  COLORREF skinItemBorder = CLR_INVALID;
  COLORREF skinSelectedBg = CLR_INVALID;
  COLORREF skinSelectedBorder = CLR_INVALID;
  COLORREF skinPressedBg = CLR_INVALID;
  COLORREF skinPressedBorder = CLR_INVALID;
  COLORREF skinSelectedText = CLR_INVALID;
  COLORREF skinSelectedMutedText = CLR_INVALID;
  COLORREF skinChipBg = CLR_INVALID;
  COLORREF skinChipBorder = CLR_INVALID;
  COLORREF skinChipText = CLR_INVALID;
  COLORREF skinChipActiveBg = CLR_INVALID;
  COLORREF skinChipActiveBorder = CLR_INVALID;
  COLORREF skinChipActiveText = CLR_INVALID;
  COLORREF skinSelectedOutline = CLR_INVALID;
  int skinSelectedAccentWidth = -1;
  float skinSelectedRingOpacity = -1.0f;
  std::wstring skinSelectedIndicator;
  float skinBorderOpacity = 1.0f;
  float skinDividerOpacity = 1.0f;
  float skinShadowOpacity = 0.0f;
  int skinShadowSize = 0;
  bool skinShadowEnabled = true;
  bool skinAnimationsEnabled = true;
  int skinShowAnimationMs = -1;
  int skinSelectionAnimationMs = -1;
  int skinHoverAnimationMs = -1;
  int skinPressAnimationMs = -1;
  int skinPageAnimationMs = -1;
  int skinFontWeight = -1;
  int skinSelectedFontWeight = -1;
  int skinLabelFontWeight = -1;
  int skinChipFontWeight = -1;
  int skinCornerRadius = -1;
  int skinHeaderCornerRadius = -1;
  int skinRowCornerRadius = -1;
  int skinBadgeCornerRadius = -1;

  // Skin overrides: layout/metrics (negative = no override).
  int skinOuterPadX = -1;
  int skinOuterPadY = -1;
  int skinHeaderPadX = -1;
  int skinHeaderPadY = -1;
  int skinHeaderGap = -1;
  int skinItemGap = -1;
  int skinItemPadX = -1;
  int skinItemPadY = -1;
  int skinLabelWidth = -1;
  int skinLabelGap = -1;
  int skinCommentGap = -1;
  int skinMinWidth = -1;
  int skinPreferredWidth = -1;
  int skinMaxWidth = -1;
  int skinMinHorizontalCardWidth = -1;
  int skinMaxHorizontalCardWidth = -1;

  bool skinLoaded = false;
};

struct SrfEngineOptions {
  bool jianpin = true;
  bool mixedPinyin = true;
  bool mixedPinyinAggressive = false;
  std::wstring learningSensitivity = L"standard";
  bool vAssist = true;
  bool uMode = false;
  bool retryOnFailure = true;
  bool showStatusNotifications = true;
};

/// 有候选时翻页键是否生效（关闭后按键将被吞掉，避免误入拼音串）。
struct SrfHotkeyOptions {
  bool enabled = false;
  UINT vk = 'A';
  UINT modifiers = TF_MOD_CONTROL | TF_MOD_ALT;
};

struct SrfInputOptions {
  bool defaultAscii = false;
  bool defaultFullShape = false;
  bool defaultChinesePunct = true;
  bool defaultFuzzyPinyin = false;
  bool defaultDoublePinyin = false;
  bool curlyPunct = true;
  bool autoPairPunct = true;
  bool numberFullwidth = false;
  bool symbolFullwidth = true;
  bool shiftSymbolTemporaryAscii = false;
  bool dateAutoFormat = true;
  bool englishWordInput = false;
  bool symbolToolbox = true;
  bool emojiInput = true;
  bool traditionalOutput = false;
  // 0=Ctrl+Shift and Ctrl+Space, 1=Ctrl+Shift, 2=Ctrl+Space, 3=disabled.
  UINT cnEnHotkey = 3;
  bool fullShapeHotkeyEnabled = false;
  bool punctHotkeyEnabled = false;
  bool fuzzyHotkeyEnabled = false;
  bool doubleHotkeyEnabled = false;
  bool shiftTapHotkeyEnabled = true;
  bool candidateNumberSelect = true;
  bool candidateLeftClick = true;
  bool candidateRightClick = true;
  bool pageMinusEqual = true;
  bool pageCommaPeriod = true;
  bool pagePgUpDown = true;
  SrfHotkeyOptions traditionalHotkey = {};
  SrfHotkeyOptions gameModeHotkey = {};
  SrfHotkeyOptions temporaryAsciiHotkey = {};
};

struct SrfClipboardOptions {
  bool backgroundEnabled = false;
  UINT maxHistoryItems = 60;
  UINT maxPinnedItems = 24;
  UINT maxTextUtf16Units = 20000;
};

struct SrfPrivacyOptions {
  bool enabled = false;
  std::vector<std::wstring> neverLearnProcessList;
  std::vector<std::wstring> neverClipboardProcessList;
  std::vector<std::wstring> neverCandidateProcessList;
};

struct SrfScreenshotOptions {
  SrfHotkeyOptions hotkey = {};
  std::wstring saveDir;
  std::wstring format = L"png";
};

struct SrfCompatibilityOptions {
  bool fullscreenDetection = true;
  SrfFullscreenPolicy fullscreenPolicy = SrfFullscreenPolicy::ShowUi;
  SrfCommitTransport commitTransport = SrfCommitTransport::Tsf;
  bool builtinGameList = true;
  bool autoSuggestAppOptions = true;
  std::vector<std::wstring> gameProcessList;
};

struct SrfAppOptions {
  bool hasAsciiMode = false;
  bool asciiMode = false;
  bool hasHideUi = false;
  bool hideUi = false;
  bool hasInlinePreedit = false;
  bool inlinePreedit = true;
  bool hasEnhancedPosition = false;
  bool enhancedPosition = true;
  bool hasCandidateTopmost = false;
  bool candidateTopmost = true;
  bool hasFocusPolicy = false;
  SrfFocusPolicy focusPolicy = SrfFocusPolicy::Normal;
  bool hasCommitTransport = false;
  SrfCommitTransport commitTransport = SrfCommitTransport::Tsf;
  bool hasGameProfile = false;
  bool gameCompactProfile = false;
  bool hasOverlayAnchor = false;
  SrfOverlayAnchor overlayAnchor = SrfOverlayAnchor::Auto;
  bool hasOverlayOffsetX = false;
  int overlayOffsetX = 0;
  bool hasOverlayOffsetY = false;
  int overlayOffsetY = 0;
  bool hasOverlayScale = false;
  UINT overlayScalePercent = 100;
  bool hasOverlayMonitor = false;
  std::wstring overlayMonitor = L"auto";
  bool hasOverlayBackend = false;
  SrfOverlayBackend overlayBackend = SrfOverlayBackend::Auto;
};

enum class SrfNotificationKind { Ime, FullShape, Punctuation, Fuzzy, DoublePinyin, AppOptions, Engine };

struct CaseInsensitiveWHash {
  size_t operator()(const std::wstring& value) const noexcept {
    std::wstring lowered = value;
    std::transform(lowered.begin(), lowered.end(), lowered.begin(), towlower);
    return std::hash<std::wstring>{}(lowered);
  }
};

struct CaseInsensitiveWEqual {
  bool operator()(const std::wstring& lhs, const std::wstring& rhs) const noexcept {
    if (lhs.size() != rhs.size()) return false;
    for (size_t i = 0; i < lhs.size(); ++i) {
      if (towlower(lhs[i]) != towlower(rhs[i])) return false;
    }
    return true;
  }
};

struct SrfConfig {
  SrfUIStyle style = {};
  SrfEngineOptions engine = {};
  SrfInputOptions input = {};
  SrfClipboardOptions clipboard = {};
  SrfPrivacyOptions privacy = {};
  SrfScreenshotOptions screenshot = {};
  SrfCompatibilityOptions compatibility = {};
  bool globalAscii = false;
  bool showNotifications = true;
  UINT showNotificationsTimeMs = 1200;
  std::vector<std::wstring> notificationKinds;
  std::unordered_map<std::wstring, SrfAppOptions, CaseInsensitiveWHash, CaseInsensitiveWEqual>
      appOptions;

  bool ShouldShowNotification(SrfNotificationKind kind) const {
    if (!showNotifications) return false;
    if (notificationKinds.empty()) return true;

    auto lower = [](std::wstring value) {
      std::transform(value.begin(), value.end(), value.begin(), towlower);
      return value;
    };

    const std::wstring kindName = [&]() {
      switch (kind) {
        case SrfNotificationKind::Ime:
          return std::wstring(L"ime");
        case SrfNotificationKind::FullShape:
          return std::wstring(L"full_shape");
        case SrfNotificationKind::Punctuation:
          return std::wstring(L"punct");
        case SrfNotificationKind::Fuzzy:
          return std::wstring(L"fuzzy");
        case SrfNotificationKind::DoublePinyin:
          return std::wstring(L"double");
        case SrfNotificationKind::AppOptions:
          return std::wstring(L"app");
        case SrfNotificationKind::Engine:
          return std::wstring(L"engine");
      }
      return std::wstring();
    }();

    const std::wstring loweredKind = lower(kindName);
    for (const auto& entry : notificationKinds) {
      const std::wstring lowered = lower(entry);
      if (lowered == L"always" || lowered == loweredKind) return true;
    }
    return false;
  }
};

inline const SrfAppOptions* FindAppOptions(const SrfConfig& config,
                                           const std::wstring& appPath) {
  if (appPath.empty()) return nullptr;

  // A path-specific rule must win over a generic executable-name rule. The
  // map itself compares case-insensitively, matching Windows path semantics.
  auto it = config.appOptions.find(appPath);
  if (it != config.appOptions.end()) return &it->second;

  const auto normalizePath = [](std::wstring value) {
    std::replace(value.begin(), value.end(), L'/', L'\\');
    if (value.rfind(L"\\\\?\\", 0) == 0) value.erase(0, 4);
    std::transform(value.begin(), value.end(), value.begin(), towlower);
    return value;
  };
  const std::wstring normalizedPath = normalizePath(appPath);
  if (appPath.find_first_of(L"\\/") != std::wstring::npos) {
    for (const auto& entry : config.appOptions) {
      if (entry.first.find_first_of(L"\\/") == std::wstring::npos) continue;
      if (normalizePath(entry.first) == normalizedPath) return &entry.second;
    }
  }

  const size_t separator = appPath.find_last_of(L"\\/");
  if (separator == std::wstring::npos || separator + 1 >= appPath.size()) return nullptr;

  const std::wstring baseName = appPath.substr(separator + 1);
  it = config.appOptions.find(baseName);
  return it != config.appOptions.end() ? &it->second : nullptr;
}
