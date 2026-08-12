// Candidate-window theme colors and contrast helpers.

struct CandidateColors {
  COLORREF windowBg = RGB(248, 250, 252);
  COLORREF windowBgTo = CLR_INVALID;  // Gradient end color
  COLORREF headerBg = RGB(241, 245, 249);
  COLORREF headerBgTo = CLR_INVALID;  // Gradient end color
  COLORREF border = RGB(203, 213, 225);
  COLORREF divider = CLR_INVALID;     // Divider line color
  COLORREF itemBg = RGB(255, 255, 255);
  COLORREF itemBorder = RGB(226, 232, 240);
  COLORREF hoverBg = RGB(241, 247, 255);
  COLORREF hoverBorder = RGB(191, 219, 254);
  COLORREF selectedBg = RGB(234, 243, 255);
  COLORREF selectedBorder = RGB(187, 215, 255);
  COLORREF pressedBg = RGB(219, 234, 254);
  COLORREF pressedBorder = RGB(147, 197, 253);
  COLORREF text = RGB(15, 23, 42);
  COLORREF mutedText = RGB(100, 116, 139);
  COLORREF selectedText = RGB(15, 23, 42);
  COLORREF selectedMutedText = RGB(71, 85, 105);
  COLORREF selectedOutline = RGB(37, 99, 235);
  COLORREF badgeBg = RGB(241, 245, 249);
  COLORREF badgeBorder = RGB(203, 213, 225);
  COLORREF badgeText = RGB(51, 65, 85);
  COLORREF chipBg = RGB(248, 250, 252);
  COLORREF chipBorder = RGB(226, 232, 240);
  COLORREF chipText = RGB(100, 116, 139);
  COLORREF chipActiveBg = RGB(239, 246, 255);
  COLORREF chipActiveBorder = RGB(191, 219, 254);
  COLORREF chipActiveText = RGB(29, 78, 216);
  float borderOpacity = 0.86f;
  float dividerOpacity = 0.48f;
  float shadowOpacity = 0.08f;
  int shadowSize = 8;
  bool shadowEnabled = true;
  bool isMist = false;  // Soft mist gradient; no backdrop blur.
};

COLORREF AlphaBlendColor(COLORREF fg, COLORREF bg, float alpha) {
  const int a = static_cast<int>(std::clamp(alpha, 0.0f, 1.0f) * 255);
  const int r = (GetRValue(fg) * a + GetRValue(bg) * (255 - a)) / 255;
  const int g = (GetGValue(fg) * a + GetGValue(bg) * (255 - a)) / 255;
  const int b = (GetBValue(fg) * a + GetBValue(bg) * (255 - a)) / 255;
  return RGB(r, g, b);
}

double ToLinearLumaChannel(int channel) {
  const double s = static_cast<double>(channel) / 255.0;
  return s <= 0.04045 ? (s / 12.92) : pow((s + 0.055) / 1.055, 2.4);
}

double RelativeLuminance(COLORREF color) {
  const double r = ToLinearLumaChannel(GetRValue(color));
  const double g = ToLinearLumaChannel(GetGValue(color));
  const double b = ToLinearLumaChannel(GetBValue(color));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

double ContrastRatio(COLORREF a, COLORREF b) {
  const double l1 = RelativeLuminance(a);
  const double l2 = RelativeLuminance(b);
  const double hi = std::max(l1, l2);
  const double lo = std::min(l1, l2);
  return (hi + 0.05) / (lo + 0.05);
}

double MinContrastAgainst(COLORREF text, const COLORREF* backgrounds, size_t count) {
  if (!backgrounds || count == 0) return 0.0;
  double minRatio = ContrastRatio(text, backgrounds[0]);
  for (size_t i = 1; i < count; ++i) {
    minRatio = std::min(minRatio, ContrastRatio(text, backgrounds[i]));
  }
  return minRatio;
}

COLORREF EnsureReadableTextColor(COLORREF preferred, const COLORREF* backgrounds, size_t count,
                                 double minContrast) {
  if (!backgrounds || count == 0) return preferred;
  const double preferredMin = MinContrastAgainst(preferred, backgrounds, count);
  if (preferredMin >= minContrast) return preferred;

  const COLORREF white = RGB(255, 255, 255);
  const COLORREF black = RGB(0, 0, 0);
  const double whiteMin = MinContrastAgainst(white, backgrounds, count);
  const double blackMin = MinContrastAgainst(black, backgrounds, count);

  if (whiteMin >= minContrast || blackMin >= minContrast) {
    return whiteMin >= blackMin ? white : black;
  }
  // 两者都达不到时，选择最不差的。
  return whiteMin >= blackMin ? white : black;
}

CandidateColors ResolveColors(const SrfUIStyle& style) {
  CandidateColors colors = {};
  // Set divider defaults based on theme
  if (style.themeMode == SrfThemeMode::Dark) {
    colors.windowBg = RGB(31, 33, 37);
    colors.headerBg = RGB(44, 47, 54);
    colors.border = RGB(83, 88, 99);
    colors.divider = RGB(71, 76, 86);
    colors.itemBg = RGB(42, 45, 51);
    colors.itemBorder = RGB(71, 76, 86);
    colors.hoverBg = RGB(55, 60, 69);
    colors.hoverBorder = RGB(113, 128, 148);
    colors.selectedBg = RGB(103, 136, 222);
    colors.selectedBorder = RGB(131, 159, 234);
    colors.pressedBg = RGB(82, 113, 194);
    colors.pressedBorder = RGB(116, 146, 226);
    colors.text = RGB(241, 244, 249);
    colors.mutedText = RGB(183, 191, 204);
    colors.selectedText = RGB(255, 255, 255);
    colors.selectedMutedText = RGB(228, 235, 251);
    colors.badgeBg = RGB(63, 67, 76);
    colors.badgeBorder = RGB(110, 116, 128);
    colors.badgeText = RGB(235, 240, 247);
    colors.chipBg = RGB(63, 67, 76);
    colors.chipBorder = RGB(110, 116, 128);
    colors.chipText = RGB(235, 240, 247);
    colors.chipActiveBg = RGB(103, 136, 222);
    colors.chipActiveBorder = RGB(131, 159, 234);
    colors.chipActiveText = RGB(255, 255, 255);
    colors.selectedOutline = RGB(15, 20, 32);
  } else if (style.themeMode == SrfThemeMode::HighContrast) {
    const COLORREF highlight = GetSysColor(COLOR_HIGHLIGHT);
    const COLORREF highlightText = GetSysColor(COLOR_HIGHLIGHTTEXT);
    const COLORREF hotlight = GetSysColor(COLOR_HOTLIGHT);
    colors.windowBg = RGB(0, 0, 0);
    colors.headerBg = RGB(0, 0, 0);
    colors.border = RGB(255, 255, 255);
    colors.divider = RGB(128, 128, 128);
    colors.itemBg = RGB(0, 0, 0);
    colors.itemBorder = RGB(255, 255, 255);
    colors.hoverBg = RGB(28, 28, 28);
    colors.hoverBorder = hotlight == RGB(0, 0, 0) ? RGB(255, 255, 255) : hotlight;
    colors.selectedBg = highlight;
    colors.selectedBorder = RGB(255, 255, 255);
    colors.pressedBg = AlphaBlendColor(highlight, RGB(0, 0, 0), 0.82f);
    colors.pressedBorder = RGB(255, 255, 255);
    colors.text = RGB(255, 255, 255);
    colors.mutedText = RGB(214, 214, 214);
    colors.selectedText = highlightText;
    colors.selectedMutedText = highlightText;
    colors.badgeBg = RGB(0, 0, 0);
    colors.badgeBorder = RGB(255, 255, 255);
    colors.badgeText = RGB(255, 255, 255);
    colors.chipBg = RGB(0, 0, 0);
    colors.chipBorder = RGB(255, 255, 255);
    colors.chipText = RGB(255, 255, 255);
    colors.chipActiveBg = highlight;
    colors.chipActiveBorder = RGB(255, 255, 255);
    colors.chipActiveText = highlightText;
    colors.selectedOutline = RGB(255, 255, 255);
  } else {
    // Light: divider defaults
    colors.divider = RGB(226, 232, 240);
    colors.selectedOutline = RGB(37, 99, 235);
  }

  // Apply material effects
  const SrfCandidateMaterial material = style.candidateMaterial;
  if (material == SrfCandidateMaterial::Gradient) {
    if (style.themeMode == SrfThemeMode::Dark) {
      colors.windowBgTo = RGB(37, 39, 45);
      colors.headerBgTo = RGB(50, 54, 62);
    } else if (style.themeMode != SrfThemeMode::HighContrast) {
      colors.windowBgTo = RGB(241, 245, 249);
      colors.headerBgTo = RGB(226, 232, 240);
    }
  } else if (material == SrfCandidateMaterial::Mist) {
    colors.isMist = true;
    colors.shadowOpacity = 0.06f;
    colors.shadowSize = 5;
    if (style.themeMode == SrfThemeMode::Dark) {
      colors.windowBg = RGB(35, 37, 42);
      colors.windowBgTo = RGB(28, 30, 34);
      colors.headerBg = RGB(42, 45, 52);
      colors.headerBgTo = RGB(38, 40, 47);
    } else if (style.themeMode != SrfThemeMode::HighContrast) {
      colors.windowBg = RGB(248, 250, 252);
      colors.windowBgTo = RGB(241, 245, 249);
      colors.headerBg = RGB(241, 245, 249);
      colors.headerBgTo = RGB(226, 232, 240);
    }
  }

  // Apply skin overrides
  if (style.skinLoaded) {
    if (style.skinWindowBg != CLR_INVALID) colors.windowBg = style.skinWindowBg;
    if (style.skinWindowBgTo != CLR_INVALID) colors.windowBgTo = style.skinWindowBgTo;
    if (style.skinHeaderBg != CLR_INVALID) colors.headerBg = style.skinHeaderBg;
    if (style.skinHeaderBgTo != CLR_INVALID) colors.headerBgTo = style.skinHeaderBgTo;
    if (style.skinBorder != CLR_INVALID) colors.border = style.skinBorder;
    if (style.skinDivider != CLR_INVALID) colors.divider = style.skinDivider;
    if (style.skinText != CLR_INVALID) colors.text = style.skinText;
    if (style.skinMutedText != CLR_INVALID) colors.mutedText = style.skinMutedText;
    if (style.skinBadgeBg != CLR_INVALID) colors.badgeBg = style.skinBadgeBg;
    if (style.skinBadgeBorder != CLR_INVALID) colors.badgeBorder = style.skinBadgeBorder;
    if (style.skinBadgeText != CLR_INVALID) colors.badgeText = style.skinBadgeText;
    if (style.skinHoverBg != CLR_INVALID) colors.hoverBg = style.skinHoverBg;
    if (style.skinHoverBorder != CLR_INVALID) colors.hoverBorder = style.skinHoverBorder;
    if (style.skinItemBg != CLR_INVALID) colors.itemBg = style.skinItemBg;
    if (style.skinItemBorder != CLR_INVALID) colors.itemBorder = style.skinItemBorder;
    if (style.skinSelectedBg != CLR_INVALID) colors.selectedBg = style.skinSelectedBg;
    if (style.skinSelectedBorder != CLR_INVALID) colors.selectedBorder = style.skinSelectedBorder;
    if (style.skinPressedBg != CLR_INVALID) colors.pressedBg = style.skinPressedBg;
    if (style.skinPressedBorder != CLR_INVALID) colors.pressedBorder = style.skinPressedBorder;
    if (style.skinSelectedText != CLR_INVALID) colors.selectedText = style.skinSelectedText;
    if (style.skinSelectedMutedText != CLR_INVALID) colors.selectedMutedText = style.skinSelectedMutedText;
    if (style.skinChipBg != CLR_INVALID) colors.chipBg = style.skinChipBg;
    if (style.skinChipBorder != CLR_INVALID) colors.chipBorder = style.skinChipBorder;
    if (style.skinChipText != CLR_INVALID) colors.chipText = style.skinChipText;
    if (style.skinChipActiveBg != CLR_INVALID) colors.chipActiveBg = style.skinChipActiveBg;
    if (style.skinChipActiveBorder != CLR_INVALID) colors.chipActiveBorder = style.skinChipActiveBorder;
    if (style.skinChipActiveText != CLR_INVALID) colors.chipActiveText = style.skinChipActiveText;
    if (style.skinSelectedOutline != CLR_INVALID) colors.selectedOutline = style.skinSelectedOutline;
    colors.borderOpacity = style.skinBorderOpacity;
    colors.dividerOpacity = style.skinDividerOpacity;
    colors.shadowOpacity = style.skinShadowOpacity;
    colors.shadowSize = style.skinShadowSize;
    colors.shadowEnabled = style.skinShadowEnabled;
  }

  // If divider not set, derive from border
  if (colors.divider == CLR_INVALID) {
    colors.divider = AlphaBlendColor(colors.border, colors.windowBg, 0.5f);
  }

  // 皮肤可能配置出低对比配色。这里仅在可读性不足时自动矫正文本颜色。
  const COLORREF normalBackgrounds[] = {colors.windowBg, colors.itemBg, colors.hoverBg, colors.pressedBg};
  colors.text = EnsureReadableTextColor(colors.text, normalBackgrounds, _countof(normalBackgrounds), 4.2);
  colors.mutedText =
      EnsureReadableTextColor(colors.mutedText, normalBackgrounds, _countof(normalBackgrounds), 3.2);
  colors.badgeText =
      EnsureReadableTextColor(colors.badgeText, normalBackgrounds, _countof(normalBackgrounds), 3.6);

  const COLORREF selectedBackgrounds[] = {colors.selectedBg, colors.selectedBorder};
  colors.selectedText = EnsureReadableTextColor(colors.selectedText, selectedBackgrounds,
                                                _countof(selectedBackgrounds), 4.2);
  colors.selectedMutedText = EnsureReadableTextColor(colors.selectedMutedText, selectedBackgrounds,
                                                     _countof(selectedBackgrounds), 3.2);
  const COLORREF chipBackgrounds[] = {colors.chipBg, colors.chipActiveBg};
  colors.chipText = EnsureReadableTextColor(colors.chipText, chipBackgrounds, _countof(chipBackgrounds), 3.6);
  colors.chipActiveText =
      EnsureReadableTextColor(colors.chipActiveText, chipBackgrounds, _countof(chipBackgrounds), 4.2);

  return colors;
}
