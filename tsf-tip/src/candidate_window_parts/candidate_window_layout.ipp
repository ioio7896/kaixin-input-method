// Candidate-window layout metrics, DPI scaling, and horizontal paging helpers.

struct LayoutSpec {
  int outerPadX = 8;
  int outerPadY = 6;
  int headerPadX = 10;
  int headerPadY = 5;
  int headerGap = 4;
  int itemGap = 2;
  int itemPadX = 8;
  int itemPadY = 5;
  int labelWidth = 26;
  int labelGap = 6;
  int commentGap = 3;
  int minWidth = 220;
  int preferredWidth = 330;
  int maxWidth = 560;
  int minHorizontalCardWidth = 80;
  int maxHorizontalCardWidth = 150;
  int cornerRadius = 8;
  int itemRadius = 6;
  int badgeRadius = 5;
};

void ApplySkinLayoutOverrides(const SrfUIStyle& style, LayoutSpec& spec) {
  if (!style.skinLoaded) return;
  auto applyIf = [](int v, int& out) {
    if (v >= 0) out = v;
  };

  applyIf(style.skinOuterPadX, spec.outerPadX);
  applyIf(style.skinOuterPadY, spec.outerPadY);
  applyIf(style.skinHeaderPadX, spec.headerPadX);
  applyIf(style.skinHeaderPadY, spec.headerPadY);
  applyIf(style.skinHeaderGap, spec.headerGap);
  applyIf(style.skinItemGap, spec.itemGap);
  applyIf(style.skinItemPadX, spec.itemPadX);
  applyIf(style.skinItemPadY, spec.itemPadY);
  applyIf(style.skinLabelWidth, spec.labelWidth);
  applyIf(style.skinLabelGap, spec.labelGap);
  applyIf(style.skinCommentGap, spec.commentGap);
  applyIf(style.skinMinWidth, spec.minWidth);
  applyIf(style.skinPreferredWidth, spec.preferredWidth);
  applyIf(style.skinMaxWidth, spec.maxWidth);
  applyIf(style.skinMinHorizontalCardWidth, spec.minHorizontalCardWidth);
  applyIf(style.skinMaxHorizontalCardWidth, spec.maxHorizontalCardWidth);

  // Keep sane ordering
  spec.minWidth = std::max(120, spec.minWidth);
  spec.preferredWidth = std::max(spec.minWidth, spec.preferredWidth);
  spec.maxWidth = std::max(spec.preferredWidth, spec.maxWidth);
  spec.minHorizontalCardWidth = std::max(64, spec.minHorizontalCardWidth);
  spec.maxHorizontalCardWidth = std::max(spec.minHorizontalCardWidth, spec.maxHorizontalCardWidth);
}

void ApplyDensityOverrides(const SrfUIStyle& style, LayoutSpec& spec) {
  if (style.candidateDensity == SrfCandidateDensity::Compact) {
    spec.outerPadX = std::max(4, spec.outerPadX - 2);
    spec.outerPadY = std::max(4, spec.outerPadY - 2);
    spec.headerPadX = std::max(6, spec.headerPadX - 2);
    spec.headerPadY = std::max(4, spec.headerPadY - 1);
    spec.headerGap = std::max(2, spec.headerGap - 1);
    spec.itemGap = std::max(1, spec.itemGap - 1);
    spec.itemPadX = std::max(4, spec.itemPadX - 2);
    spec.itemPadY = std::max(3, spec.itemPadY - 1);
    spec.labelWidth = std::max(20, spec.labelWidth - 2);
    spec.labelGap = std::max(4, spec.labelGap - 1);
    spec.commentGap = std::max(1, spec.commentGap - 1);
    spec.preferredWidth = std::max(spec.minWidth, spec.preferredWidth - 24);
    spec.maxWidth = std::max(spec.preferredWidth, spec.maxWidth - 48);
    spec.minHorizontalCardWidth = std::max(64, spec.minHorizontalCardWidth - 8);
    spec.maxHorizontalCardWidth = std::max(spec.minHorizontalCardWidth, spec.maxHorizontalCardWidth - 18);
  } else if (style.candidateDensity == SrfCandidateDensity::Comfortable) {
    spec.outerPadX += 2;
    spec.outerPadY += 1;
    spec.headerPadX += 2;
    spec.headerPadY += 1;
    spec.headerGap += 1;
    spec.itemGap += 1;
    spec.itemPadX += 1;
    spec.itemPadY += 1;
    spec.labelWidth += 2;
    spec.labelGap += 1;
    spec.commentGap += 1;
    spec.preferredWidth += 12;
    spec.maxWidth += 20;
  }
}

LayoutSpec ResolveLayoutSpec(const SrfUIStyle& style) {
  LayoutSpec spec = {};
  const auto variant = style.candidateLayoutVariant;
  if (variant == SrfCandidateLayoutVariant::Compact) {
    if (style.candidateHorizontal) {
      spec.outerPadX = 7;
      spec.outerPadY = 5;
      spec.headerPadX = 8;
      spec.headerPadY = 5;
      spec.headerGap = 3;
      spec.itemGap = 1;
      spec.itemPadX = 5;
      spec.itemPadY = 4;
      spec.labelWidth = 22;
      spec.labelGap = 5;
      spec.commentGap = 2;
      spec.minWidth = 180;
      spec.preferredWidth = 270;
      spec.maxWidth = 460;
      spec.minHorizontalCardWidth = 72;
      spec.maxHorizontalCardWidth = 138;
      spec.cornerRadius = 8;
      spec.itemRadius = 7;
      spec.badgeRadius = 5;
    } else {
      spec.outerPadX = 10;
      spec.outerPadY = 7;
      spec.headerPadX = 12;
      spec.headerPadY = 6;
      spec.headerGap = 5;
      spec.itemGap = 3;
      spec.itemPadX = 9;
      spec.itemPadY = 6;
      spec.labelWidth = 28;
      spec.labelGap = 8;
      spec.commentGap = 4;
      spec.minWidth = 230;
      spec.preferredWidth = 350;
      spec.maxWidth = 580;
      spec.minHorizontalCardWidth = 80;
      spec.maxHorizontalCardWidth = 150;
      spec.cornerRadius = 8;
      spec.itemRadius = 6;
      spec.badgeRadius = 5;
    }
  } else if (variant == SrfCandidateLayoutVariant::Card) {
    spec.outerPadX = 10;
    spec.outerPadY = 8;
    spec.headerPadX = 12;
    spec.headerPadY = 8;
    spec.headerGap = 6;
    spec.itemGap = 3;
    spec.itemPadX = 8;
    spec.itemPadY = 6;
    spec.labelWidth = 28;
    spec.labelGap = 8;
    spec.commentGap = 3;
    spec.minWidth = 240;
    spec.preferredWidth = 360;
    spec.maxWidth = 600;
    spec.minHorizontalCardWidth = 92;
    spec.maxHorizontalCardWidth = 168;
    spec.cornerRadius = 12;
    spec.itemRadius = 10;
    spec.badgeRadius = 7;
  }
  ApplySkinLayoutOverrides(style, spec);
  ApplyDensityOverrides(style, spec);
  spec.minWidth = std::max(120, spec.minWidth);
  spec.preferredWidth = std::max(spec.minWidth, spec.preferredWidth);
  spec.maxWidth = std::max(spec.preferredWidth, spec.maxWidth);
  spec.minHorizontalCardWidth = std::max(64, spec.minHorizontalCardWidth);
  spec.maxHorizontalCardWidth = std::max(spec.minHorizontalCardWidth, spec.maxHorizontalCardWidth);
  return spec;
}

int ScaleForDpi(int value, UINT dpi) {
  return MulDiv(value, static_cast<int>(dpi == 0 ? 96u : dpi), 96);
}

int StrokeWidthForDpi(UINT dpi) {
  return (dpi == 0 ? 96u : dpi) >= 144u ? 2 : 1;
}

int SnapGdiRadiusForDpi(int radius, UINT dpi) {
  const UINT resolvedDpi = dpi == 0 ? 96u : dpi;
  (void)resolvedDpi;
  radius = std::max(0, radius);
  if (radius <= 1) return radius;
  return (radius + 1) & ~1;
}

int HorizontalCompactDeltaForDpi(const SrfUIStyle& style, UINT dpi) {
  const UINT resolvedDpi = dpi == 0 ? 96u : dpi;
  int logicalDelta = 0;
  if (style.candidateHorizontalCompact) logicalDelta += resolvedDpi >= 144u ? 1 : 2;
  // Keep skin spacing compact on standard DPI, but avoid over-compressing high-DPI screens.
  if (style.skinLoaded && resolvedDpi < 144u) logicalDelta += 1;
  return ScaleForDpi(logicalDelta, resolvedDpi);
}

int ResolveHorizontalCardsAreaWidth(const SrfUIStyle& style, const RECT* anchorRect, UINT dpi) {
  const LayoutSpec spec = ResolveLayoutSpec(style);
  const int outerPadX = ScaleForDpi(spec.outerPadX, dpi);
  const int minWidth = ScaleForDpi(spec.minWidth, dpi);
  const int compact1 = HorizontalCompactDeltaForDpi(style, dpi);
  const int hOuterPadX = std::max(0, outerPadX - compact1);

  const RECT work = PlacementAreaForAnchor(anchorRect, style.candidateFullscreenPlacement);
  const int screenMargin = ScaleForDpi(10, dpi);
  const int maxWidth =
      std::max(ScaleForDpi(260, dpi), static_cast<int>((work.right - work.left) - screenMargin * 2));
  const int usableMaxWidth = std::max(minWidth, maxWidth);
  return std::max(ScaleForDpi(100, dpi), usableMaxWidth - hOuterPadX * 2);
}

int ResolveHorizontalVisibleCountForArea(const SrfUIStyle& style, UINT dpi, int areaWidth,
                                         size_t remainingItems) {
  if (remainingItems == 0 || areaWidth <= 0) return 0;
  const LayoutSpec spec = ResolveLayoutSpec(style);
  const int minHorizontalCardWidth = ScaleForDpi(spec.minHorizontalCardWidth, dpi);
  const int itemGap = ScaleForDpi(spec.itemGap, dpi);
  const int compact1 = HorizontalCompactDeltaForDpi(style, dpi);
  const int hItemGap = std::max(0, itemGap - compact1);
  const int minCardWidth =
      std::max(ScaleForDpi(56, dpi), std::max(ScaleForDpi(48, dpi), minHorizontalCardWidth - compact1 * 4));
  const int maxByWidth = std::max(1, (areaWidth + hItemGap) / (minCardWidth + hItemGap));
  const int configuredMax = static_cast<int>(std::clamp(style.candidateHorizontalCount, 3u, 9u));
  const int remaining = static_cast<int>(remainingItems);
  int visible = std::min({configuredMax, remaining, maxByWidth});
  return std::max(1, visible);
}

int EstimateHorizontalItemWidthForPaging(const SrfUIStyle& style, UINT dpi,
                                         const std::wstring& text) {
  const LayoutSpec spec = ResolveLayoutSpec(style);
  const int itemPadX = ScaleForDpi(spec.itemPadX, dpi);
  const int labelWidth = ScaleForDpi(spec.labelWidth, dpi);
  const int labelGap = ScaleForDpi(spec.labelGap, dpi);
  const int compact1 = HorizontalCompactDeltaForDpi(style, dpi);
  const int hItemPadX = std::max(0, itemPadX - compact1);
  const int fontPx = std::max(ScaleForDpi(12, dpi),
                              MulDiv(static_cast<int>(style.candidateFontSize == 0
                                                          ? 14u
                                                          : style.candidateFontSize),
                                     static_cast<int>(dpi == 0 ? 96u : dpi), 72));

  const std::wstring displayText =
      AbbreviateCandidateForDisplay(text, style.candidateAbbreviateLength);
  double units = 0.0;
  for (wchar_t ch : displayText) {
    if (ch == L'\r' || ch == L'\n' || ch == L'\t') {
      ch = L' ';
    }
    if (std::iswspace(static_cast<wint_t>(ch))) {
      units += 0.35;
    } else if (ch < 0x80) {
      units += 0.58;
    } else if ((ch >= 0x3000 && ch <= 0x9FFF) || (ch >= 0xFF00 && ch <= 0xFFEF)) {
      units += 1.05;
    } else {
      units += 0.9;
    }
  }

  const int textWidth = static_cast<int>(std::ceil(units * fontPx));
  const int inlineLabelWidth = std::max(ScaleForDpi(10, dpi), labelWidth / 2);
  return inlineLabelWidth + labelGap + textWidth + hItemPadX * 2 + ScaleForDpi(4, dpi);
}

int ResolveHorizontalVisibleCountForItems(const SrfUIStyle& style, UINT dpi, int areaWidth,
                                          const std::vector<std::wstring>& items,
                                          size_t start) {
  if (start >= items.size() || areaWidth <= 0) return 0;
  const LayoutSpec spec = ResolveLayoutSpec(style);
  const int minHorizontalCardWidth = ScaleForDpi(spec.minHorizontalCardWidth, dpi);
  const int itemGap = ScaleForDpi(spec.itemGap, dpi);
  const int compact1 = HorizontalCompactDeltaForDpi(style, dpi);
  const int hItemGap = std::max(0, itemGap - compact1);
  const int minCardWidth =
      std::max(ScaleForDpi(56, dpi), std::max(ScaleForDpi(48, dpi), minHorizontalCardWidth - compact1 * 4));
  const int configuredMax = static_cast<int>(std::clamp(style.candidateHorizontalCount, 3u, 9u));
  const int remaining = static_cast<int>(items.size() - start);
  const int target = std::min(configuredMax, remaining);

  int visible = 0;
  int usedWidth = 0;
  for (int i = 0; i < target; ++i) {
    const size_t idx = start + static_cast<size_t>(i);
    const int itemWidth = std::max(minCardWidth, EstimateHorizontalItemWidthForPaging(style, dpi, items[idx]));
    const int nextWidth = usedWidth + (visible > 0 ? hItemGap : 0) + itemWidth;
    if (visible > 0 && nextWidth > areaWidth) break;
    usedWidth = nextWidth;
    ++visible;
  }
  return std::max(1, visible);
}

int ResolveHorizontalCardWidthForArea(const SrfUIStyle& style, UINT dpi, int areaWidth, int visibleCount) {
  if (visibleCount <= 0 || areaWidth <= 0) return 0;
  const LayoutSpec spec = ResolveLayoutSpec(style);
  const int minHorizontalCardWidth = ScaleForDpi(spec.minHorizontalCardWidth, dpi);
  const int itemGap = ScaleForDpi(spec.itemGap, dpi);
  const int compact1 = HorizontalCompactDeltaForDpi(style, dpi);
  const int hItemGap = std::max(0, itemGap - compact1);
  const int minCardWidth =
      std::max(ScaleForDpi(56, dpi), std::max(ScaleForDpi(48, dpi), minHorizontalCardWidth - compact1 * 4));

  const int totalGaps = hItemGap * std::max(0, visibleCount - 1);
  const int raw = std::max(1, areaWidth - totalGaps) / visibleCount;
  return std::max(raw, minCardWidth);
}

int PointSizeToPixels(UINT points, UINT dpi) {
  const UINT resolvedDpi = dpi == 0 ? 96u : dpi;
  return -MulDiv(static_cast<int>(points), static_cast<int>(resolvedDpi), 72);
}
