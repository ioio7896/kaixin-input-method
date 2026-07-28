// Font fallback, DirectWrite text rendering, and text layout caches for candidate_window.cpp.

template <typename T>
void DeleteGdiObject(T* handle) {
  if (handle) DeleteObject(handle);
}

bool TextMayNeedEmojiFallback(const std::wstring& text) {
  for (size_t i = 0; i < text.size(); ++i) {
    const wchar_t ch = text[i];
    if (ch >= 0x2600 && ch <= 0x27BF) return true;
    if (ch >= 0xE000 && ch <= 0xF8FF) return true;
    if (ch >= 0xFE00 && ch <= 0xFE0F) return true;
    if (ch >= 0xD800 && ch <= 0xDBFF && i + 1 < text.size()) {
      const wchar_t lo = text[i + 1];
      if (lo >= 0xDC00 && lo <= 0xDFFF) {
        const uint32_t codepoint =
            0x10000u + ((static_cast<uint32_t>(ch) - 0xD800u) << 10) +
            (static_cast<uint32_t>(lo) - 0xDC00u);
        if ((codepoint >= 0x1F000u && codepoint <= 0x1FAFFu) ||
            (codepoint >= 0x1FC00u && codepoint <= 0x1FFFFu)) {
          return true;
        }
      }
    }
  }
  return false;
}

struct EmojiFontCache {
  HFONT font = nullptr;
  int height = 0;
  int weight = 0;

  ~EmojiFontCache() { DeleteGdiObject(font); }

  HFONT Get(HFONT baseFont) {
    if (!baseFont) return nullptr;
    LOGFONTW lf = {};
    if (!GetObjectW(baseFont, sizeof(lf), &lf)) return nullptr;
    const int nextHeight = lf.lfHeight;
    const int nextWeight = lf.lfWeight;
    if (font && height == nextHeight && weight == nextWeight) return font;
    DeleteGdiObject(font);
    wcscpy_s(lf.lfFaceName, L"Segoe UI Emoji");
    lf.lfWeight = nextWeight;
    lf.lfQuality = CLEARTYPE_QUALITY;
    font = CreateFontIndirectW(&lf);
    height = nextHeight;
    weight = nextWeight;
    return font;
  }
};

HFONT ResolveTextFont(HFONT font, const std::wstring& text) {
  if (!TextMayNeedEmojiFallback(text)) return font;
  static EmojiFontCache emojiCache;
  HFONT emojiFont = emojiCache.Get(font);
  return emojiFont ? emojiFont : font;
}

struct DirectWriteFontSpec {
  std::wstring family = kDefaultCandidateFontFace;
  DWRITE_FONT_WEIGHT weight = DWRITE_FONT_WEIGHT_NORMAL;
  DWRITE_FONT_STYLE style = DWRITE_FONT_STYLE_NORMAL;
  DWRITE_FONT_STRETCH stretch = DWRITE_FONT_STRETCH_NORMAL;
  float sizeDip = 14.0f;
  UINT dpi = 96;
};

UINT DpiFromHdc(HDC hdc) {
  if (!hdc) return 96;
  const int dpi = GetDeviceCaps(hdc, LOGPIXELSX);
  return dpi > 0 ? static_cast<UINT>(dpi) : 96u;
}

UINT ResolveTextRenderDpi(HDC hdc, UINT requestedDpi) {
  const UINT hdcDpi = DpiFromHdc(hdc);
  const UINT resolved = requestedDpi == 0 ? hdcDpi : requestedDpi;
  if (requestedDpi != 0 && hdcDpi != 0 && std::abs(static_cast<int>(hdcDpi) -
                                                    static_cast<int>(requestedDpi)) >= 2) {
    static ULONGLONG lastLogTick = 0;
    const ULONGLONG now = GetTickCount64();
    if (lastLogTick == 0 || now < lastLogTick || now - lastLogTick >= 1000) {
      lastLogTick = now;
      wchar_t line[96] = {};
      swprintf_s(line, L"hdc_dpi=%u, requested_dpi=%u", hdcDpi, requestedDpi);
      SrfTsfPerfLog(L"candidate-window.dpi-mismatch", line);
    }
  }
  return resolved == 0 ? 96u : resolved;
}

float PixelsToDips(float px, UINT dpi) {
  return px * 96.0f / static_cast<float>(dpi == 0 ? 96u : dpi);
}

float DipsToPixels(float dip, UINT dpi) {
  return dip * static_cast<float>(dpi == 0 ? 96u : dpi) / 96.0f;
}

D2D1_RECT_F PixelRectToDipRect(const RECT& rect, UINT dpi) {
  return {PixelsToDips(static_cast<float>(rect.left), dpi),
          PixelsToDips(static_cast<float>(rect.top), dpi),
          PixelsToDips(static_cast<float>(rect.right), dpi),
          PixelsToDips(static_cast<float>(rect.bottom), dpi)};
}

bool EqualInsensitive(const std::wstring& a, const std::wstring& b) {
  if (a.size() != b.size()) return false;
  for (size_t i = 0; i < a.size(); ++i) {
    if (towlower(a[i]) != towlower(b[i])) return false;
  }
  return true;
}

DirectWriteFontSpec DirectWriteFontSpecFromHfont(HFONT font, UINT dpi) {
  DirectWriteFontSpec spec = {};
  spec.dpi = dpi == 0 ? 96u : dpi;
  if (!font) return spec;
  LOGFONTW lf = {};
  if (!GetObjectW(font, sizeof(lf), &lf)) return spec;
  if (lf.lfFaceName[0] != L'\0') spec.family = lf.lfFaceName;
  const int resolvedWeight = lf.lfWeight > 0 ? lf.lfWeight : FW_NORMAL;
  spec.weight = static_cast<DWRITE_FONT_WEIGHT>(std::clamp(resolvedWeight, 100, 999));
  spec.style = lf.lfItalic ? DWRITE_FONT_STYLE_ITALIC : DWRITE_FONT_STYLE_NORMAL;
  spec.sizeDip =
      PixelsToDips(static_cast<float>(std::max(1, std::abs(static_cast<int>(lf.lfHeight)))),
                   spec.dpi);
  return spec;
}

D2D1_COLOR_F ToD2DColor(COLORREF color) {
  return {GetRValue(color) / 255.0f, GetGValue(color) / 255.0f,
          GetBValue(color) / 255.0f, 1.0f};
}

DWRITE_TEXT_ALIGNMENT ResolveDirectWriteTextAlignment(UINT format) {
  if (format & DT_RIGHT) return DWRITE_TEXT_ALIGNMENT_TRAILING;
  if (format & DT_CENTER) return DWRITE_TEXT_ALIGNMENT_CENTER;
  return DWRITE_TEXT_ALIGNMENT_LEADING;
}

DWRITE_PARAGRAPH_ALIGNMENT ResolveDirectWriteParagraphAlignment(UINT format) {
  if (format & DT_BOTTOM) return DWRITE_PARAGRAPH_ALIGNMENT_FAR;
  if (format & DT_VCENTER) return DWRITE_PARAGRAPH_ALIGNMENT_CENTER;
  return DWRITE_PARAGRAPH_ALIGNMENT_NEAR;
}

DWRITE_WORD_WRAPPING ResolveDirectWriteWrapping(UINT format) {
  if (format & DT_SINGLELINE) return DWRITE_WORD_WRAPPING_NO_WRAP;
  return (format & DT_WORDBREAK) ? DWRITE_WORD_WRAPPING_WRAP
                                 : DWRITE_WORD_WRAPPING_NO_WRAP;
}

D2D1_DRAW_TEXT_OPTIONS DirectWriteDrawOptions() {
  D2D1_DRAW_TEXT_OPTIONS options = D2D1_DRAW_TEXT_OPTIONS_CLIP;
#ifdef D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT
  options = static_cast<D2D1_DRAW_TEXT_OPTIONS>(
      options | D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
#endif
  return options;
}

class DirectTextRenderer {
 public:
  void ConfigureCustomFont(const std::filesystem::path& path, const std::wstring& family) {
    if (path == m_customFontPath && family == m_customFontFamily) return;
    m_customFontPath = path;
    m_customFontFamily = family;
    m_customCollection.Reset();
    m_customFamilyAvailable = false;
    ++m_customGeneration;
    ClearTextCaches();

    if (path.empty() || family.empty() || !EnsureFactories()) return;

    Microsoft::WRL::ComPtr<IDWriteFactory3> factory3;
    if (FAILED(m_dwriteFactory.As(&factory3)) || !factory3) return;

    Microsoft::WRL::ComPtr<IDWriteFontFile> fontFile;
    HRESULT hr = m_dwriteFactory->CreateFontFileReference(path.c_str(), nullptr, &fontFile);
    if (FAILED(hr) || !fontFile) return;

    Microsoft::WRL::ComPtr<IDWriteFontSetBuilder> builder;
    hr = factory3->CreateFontSetBuilder(&builder);
    if (FAILED(hr) || !builder) return;

    Microsoft::WRL::ComPtr<IDWriteFontSetBuilder1> builder1;
    if (SUCCEEDED(builder.As(&builder1)) && builder1) {
      hr = builder1->AddFontFile(fontFile.Get());
      if (FAILED(hr)) return;
    } else {
      Microsoft::WRL::ComPtr<IDWriteFontFaceReference> faceReference;
      hr = factory3->CreateFontFaceReference(fontFile.Get(), 0,
                                             DWRITE_FONT_SIMULATIONS_NONE,
                                             &faceReference);
      if (FAILED(hr) || !faceReference) return;
      hr = builder->AddFontFaceReference(faceReference.Get());
      if (FAILED(hr)) return;
    }

    Microsoft::WRL::ComPtr<IDWriteFontSet> fontSet;
    hr = builder->CreateFontSet(&fontSet);
    if (FAILED(hr) || !fontSet) return;

    Microsoft::WRL::ComPtr<IDWriteFontCollection1> collection;
    hr = factory3->CreateFontCollectionFromFontSet(fontSet.Get(), &collection);
    if (FAILED(hr) || !collection) return;

    m_customCollection = collection;
    m_customFamilyAvailable =
        IsFamilyAvailableInCollection(m_customCollection.Get(), m_customFontFamily);
    if (!m_customFamilyAvailable) {
      m_customCollection.Reset();
    }
  }

  bool MeasureSingleLine(HDC hdc, HFONT font, const std::wstring& text, SIZE* out,
                         UINT dpiOverride = 0) {
    if (!out || !hdc || !font || text.empty()) return false;
    const UINT dpi = ResolveTextRenderDpi(hdc, dpiOverride);
    Microsoft::WRL::ComPtr<IDWriteTextFormat> format;
    std::wstring formatKey;
    if (!CreateFormat(font, DT_LEFT | DT_SINGLELINE, dpi, &format, &formatKey)) return false;

    Microsoft::WRL::ComPtr<IDWriteTextLayout> layout;
    constexpr float kMeasureMaxPx = 65535.0f;
    const float maxDip = PixelsToDips(kMeasureMaxPx, dpi);
    if (!CreateTextLayout(text, format.Get(), formatKey, maxDip, maxDip,
                          DT_LEFT | DT_SINGLELINE, &layout)) {
      return false;
    }

    DWRITE_TEXT_METRICS metrics = {};
    HRESULT hr = layout->GetMetrics(&metrics);
    if (FAILED(hr)) return false;
    out->cx = std::max(
        0, static_cast<int>(std::ceil(DipsToPixels(metrics.widthIncludingTrailingWhitespace, dpi))));
    out->cy = std::max(0, static_cast<int>(std::ceil(DipsToPixels(metrics.height, dpi))));
    return true;
  }

  bool MeasureWrappedHeight(HDC hdc, HFONT font, const std::wstring& text, int width,
                            int* outHeight, UINT dpiOverride = 0) {
    if (!outHeight || !hdc || !font || text.empty()) return false;
    const UINT dpi = ResolveTextRenderDpi(hdc, dpiOverride);
    Microsoft::WRL::ComPtr<IDWriteTextFormat> format;
    std::wstring formatKey;
    if (!CreateFormat(font, DT_LEFT | DT_WORDBREAK, dpi, &format, &formatKey)) return false;

    Microsoft::WRL::ComPtr<IDWriteTextLayout> layout;
    constexpr float kMeasureMaxPx = 65535.0f;
    const float layoutWidth = PixelsToDips(static_cast<float>(std::max(1, width)), dpi);
    const float maxDip = PixelsToDips(kMeasureMaxPx, dpi);
    if (!CreateTextLayout(text, format.Get(), formatKey, layoutWidth, maxDip,
                          DT_LEFT | DT_WORDBREAK, &layout)) {
      return false;
    }

    DWRITE_TEXT_METRICS metrics = {};
    HRESULT hr = layout->GetMetrics(&metrics);
    if (FAILED(hr)) return false;
    *outHeight = std::max(0, static_cast<int>(std::ceil(DipsToPixels(metrics.height, dpi))));
    return true;
  }

  bool DrawTextBlock(HDC hdc, HFONT font, COLORREF color, const std::wstring& text,
                     RECT rect, UINT formatFlags, UINT dpiOverride = 0) {
    if (!hdc || !font || text.empty()) return false;
    const int width = rect.right - rect.left;
    const int height = rect.bottom - rect.top;
    if (width <= 0 || height <= 0) return true;

    const UINT dpi = ResolveTextRenderDpi(hdc, dpiOverride);
    if (!EnsureRenderTarget(hdc, rect, dpi)) return false;
    Microsoft::WRL::ComPtr<IDWriteTextFormat> format;
    std::wstring formatKey;
    if (!CreateFormat(font, formatFlags, dpi, &format, &formatKey)) return false;

    Microsoft::WRL::ComPtr<IDWriteTextLayout> layout;
    const D2D1_RECT_F dipRect = PixelRectToDipRect(rect, dpi);
    const float layoutWidth = std::max(1.0f, dipRect.right - dipRect.left);
    const float layoutHeight = std::max(1.0f, dipRect.bottom - dipRect.top);
    if (!CreateTextLayout(text, format.Get(), formatKey, layoutWidth, layoutHeight,
                          formatFlags, &layout)) {
      return false;
    }

    if (!EnsureBrush(color)) return false;
    const D2D1_POINT_2F origin = {dipRect.left, dipRect.top};
    m_renderTarget->BeginDraw();
    m_renderTarget->DrawTextLayout(origin, layout.Get(), m_brush.Get(),
                                   DirectWriteDrawOptions());
    HRESULT hr = m_renderTarget->EndDraw();
    if (hr == D2DERR_RECREATE_TARGET) {
      ResetRenderTarget();
      return false;
    }
    return SUCCEEDED(hr);
  }

 private:
  static constexpr size_t kMaxFormatCacheEntries = 32;
  static constexpr size_t kMaxLayoutCacheEntries = 256;
  static constexpr size_t kTrimmedLayoutCacheEntries = 192;

  struct LayoutCacheEntry {
    Microsoft::WRL::ComPtr<IDWriteTextLayout> layout;
    ULONGLONG tick = 0;
  };

  bool EnsureFactories() {
    if (m_dwriteFactory && m_d2dFactory) return true;
    if (!m_dwriteFactory) {
      HRESULT hr = DWriteCreateFactory(
          DWRITE_FACTORY_TYPE_SHARED, __uuidof(IDWriteFactory),
          reinterpret_cast<IUnknown**>(m_dwriteFactory.GetAddressOf()));
      if (FAILED(hr)) return false;
    }
    if (!m_d2dFactory) {
      D2D1_FACTORY_OPTIONS options = {};
      HRESULT hr = D2D1CreateFactory(
          D2D1_FACTORY_TYPE_MULTI_THREADED, __uuidof(ID2D1Factory), &options,
          reinterpret_cast<void**>(m_d2dFactory.GetAddressOf()));
      if (FAILED(hr)) return false;
    }
    return true;
  }

  bool CreateFormat(HFONT font, UINT formatFlags, UINT dpi,
                    Microsoft::WRL::ComPtr<IDWriteTextFormat>* out,
                    std::wstring* outKey) {
    if (!out || !EnsureFactories()) return false;
    const DirectWriteFontSpec spec = DirectWriteFontSpecFromHfont(font, dpi);
    Microsoft::WRL::ComPtr<IDWriteFontCollection> collection;
    bool customCollection = false;
    if (!ResolveFontCollection(spec.family, &collection, &customCollection)) return false;

    const std::wstring key = MakeFormatKey(spec, formatFlags, customCollection);
    auto cached = m_formatCache.find(key);
    if (cached != m_formatCache.end() && cached->second) {
      *out = cached->second;
      if (outKey) *outKey = key;
      return true;
    }

    HRESULT hr = m_dwriteFactory->CreateTextFormat(
        spec.family.c_str(), collection.Get(), spec.weight, spec.style, spec.stretch,
        spec.sizeDip, L"zh-CN", out->GetAddressOf());
    if (FAILED(hr) || !*out) return false;
    (*out)->SetTextAlignment(ResolveDirectWriteTextAlignment(formatFlags));
    (*out)->SetParagraphAlignment(ResolveDirectWriteParagraphAlignment(formatFlags));
    (*out)->SetWordWrapping(ResolveDirectWriteWrapping(formatFlags));
    ApplyLineSpacing(out->Get(), spec, collection.Get());

    if (m_formatCache.size() >= kMaxFormatCacheEntries) m_formatCache.clear();
    m_formatCache[key] = *out;
    if (outKey) *outKey = key;
    return true;
  }

  bool ResolveFontCollection(const std::wstring& family,
                             Microsoft::WRL::ComPtr<IDWriteFontCollection>* out,
                             bool* outCustom) {
    if (!out || !outCustom || family.empty()) return false;
    out->Reset();
    *outCustom = false;

    if (m_customCollection && m_customFamilyAvailable &&
        EqualInsensitive(family, m_customFontFamily)) {
      *out = m_customCollection;
      *outCustom = true;
      return true;
    }

    return IsSystemFamilyAvailable(family);
  }

  bool IsSystemFamilyAvailable(const std::wstring& family) {
    if (family.empty()) return false;
    auto it = m_systemFamilyAvailability.find(family);
    if (it != m_systemFamilyAvailability.end()) return it->second;

    Microsoft::WRL::ComPtr<IDWriteFontCollection> collection;
    HRESULT hr = m_dwriteFactory->GetSystemFontCollection(&collection, FALSE);
    if (FAILED(hr) || !collection) {
      return false;
    }

    const bool available = IsFamilyAvailableInCollection(collection.Get(), family, true);
    m_systemFamilyAvailability[family] = available;
    return available;
  }

  bool IsFamilyAvailableInCollection(IDWriteFontCollection* collection,
                                     const std::wstring& family,
                                     bool assumeAvailableOnFailure = false) const {
    if (!collection || family.empty()) return false;
    UINT32 index = 0;
    BOOL exists = FALSE;
    const HRESULT hr = collection->FindFamilyName(family.c_str(), &index, &exists);
    if (FAILED(hr)) return assumeAvailableOnFailure;
    return exists != FALSE;
  }

  void ApplyLineSpacing(IDWriteTextFormat* format, const DirectWriteFontSpec& spec,
                        IDWriteFontCollection* collection) {
    if (!format) return;
    DWRITE_FONT_METRICS metrics = {};
    if (!GetFontMetrics(spec, collection, &metrics) || metrics.designUnitsPerEm == 0) return;

    const float em = static_cast<float>(metrics.designUnitsPerEm);
    const float lineHeight =
        spec.sizeDip * static_cast<float>(metrics.ascent + metrics.descent + metrics.lineGap) / em;
    const float baseline = spec.sizeDip * static_cast<float>(metrics.ascent) / em;
    if (lineHeight > 0.0f && baseline > 0.0f) {
      format->SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, lineHeight, baseline);
    }
  }

  bool GetFontMetrics(const DirectWriteFontSpec& spec, IDWriteFontCollection* collection,
                      DWRITE_FONT_METRICS* metrics) {
    if (!metrics || !EnsureFactories()) return false;
    Microsoft::WRL::ComPtr<IDWriteFontCollection> systemCollection;
    IDWriteFontCollection* resolvedCollection = collection;
    if (!resolvedCollection) {
      if (FAILED(m_dwriteFactory->GetSystemFontCollection(&systemCollection, FALSE)) ||
          !systemCollection) {
        return false;
      }
      resolvedCollection = systemCollection.Get();
    }

    UINT32 familyIndex = 0;
    BOOL exists = FALSE;
    HRESULT hr = resolvedCollection->FindFamilyName(spec.family.c_str(), &familyIndex, &exists);
    if (FAILED(hr) || !exists) return false;

    Microsoft::WRL::ComPtr<IDWriteFontFamily> family;
    hr = resolvedCollection->GetFontFamily(familyIndex, &family);
    if (FAILED(hr) || !family) return false;

    Microsoft::WRL::ComPtr<IDWriteFont> font;
    hr = family->GetFirstMatchingFont(spec.weight, spec.stretch, spec.style, &font);
    if (FAILED(hr) || !font) return false;

    Microsoft::WRL::ComPtr<IDWriteFontFace> face;
    hr = font->CreateFontFace(&face);
    if (FAILED(hr) || !face) return false;

    face->GetMetrics(metrics);
    return true;
  }

  bool CreateTextLayout(const std::wstring& text, IDWriteTextFormat* format,
                        const std::wstring& formatKey, float widthDip, float heightDip,
                        UINT formatFlags,
                        Microsoft::WRL::ComPtr<IDWriteTextLayout>* out) {
    if (!out || !format || !m_dwriteFactory) return false;
    const std::wstring layoutKey = MakeLayoutKey(formatKey, text, widthDip, heightDip, formatFlags);
    auto cached = m_layoutCache.find(layoutKey);
    if (cached != m_layoutCache.end() && cached->second.layout) {
      cached->second.tick = ++m_cacheTick;
      *out = cached->second.layout;
      return true;
    }

    HRESULT hr = m_dwriteFactory->CreateTextLayout(
        text.c_str(), static_cast<UINT32>(text.size()), format, widthDip, heightDip,
        out->GetAddressOf());
    if (FAILED(hr) || !*out) return false;
    ApplyFontFallback(out->Get());
    ApplyTrimming(format, out->Get(), formatFlags);

    LayoutCacheEntry entry = {};
    entry.layout = *out;
    entry.tick = ++m_cacheTick;
    m_layoutCache[layoutKey] = entry;
    TrimLayoutCache();
    return true;
  }

  bool EnsureSystemFontFallback() {
    if (m_fontFallback) return true;
    if (!EnsureFactories()) return false;
    Microsoft::WRL::ComPtr<IDWriteFactory2> factory2;
    if (FAILED(m_dwriteFactory.As(&factory2)) || !factory2) return false;
    return SUCCEEDED(factory2->GetSystemFontFallback(&m_fontFallback)) && m_fontFallback;
  }

  void ApplyFontFallback(IDWriteTextLayout* layout) {
    if (!layout || !EnsureSystemFontFallback()) return;
    Microsoft::WRL::ComPtr<IDWriteTextLayout2> layout2;
    if (SUCCEEDED(layout->QueryInterface(IID_PPV_ARGS(&layout2))) && layout2) {
      layout2->SetFontFallback(m_fontFallback.Get());
    }
  }

  void ApplyTrimming(IDWriteTextFormat* format, IDWriteTextLayout* layout, UINT formatFlags) {
    if (!format || !layout || !(formatFlags & DT_END_ELLIPSIS) || !m_dwriteFactory) return;
    DWRITE_TRIMMING trimming = {};
    trimming.granularity = DWRITE_TRIMMING_GRANULARITY_CHARACTER;
    Microsoft::WRL::ComPtr<IDWriteInlineObject> sign;
    if (SUCCEEDED(m_dwriteFactory->CreateEllipsisTrimmingSign(format, &sign)) && sign) {
      layout->SetTrimming(&trimming, sign.Get());
    }
  }

  bool EnsureRenderTarget(HDC hdc, const RECT& preferredRect, UINT dpi) {
    return EnsureRenderTargetWithDepth(hdc, preferredRect, dpi, 0);
  }

  bool EnsureRenderTargetWithDepth(HDC hdc, const RECT& preferredRect, UINT dpi,
                                   int recreateDepth) {
    if (!EnsureFactories()) return false;
    if (!m_renderTarget) {
      D2D1_RENDER_TARGET_PROPERTIES props = {};
      props.type = D2D1_RENDER_TARGET_TYPE_DEFAULT;
      props.pixelFormat.format = DXGI_FORMAT_B8G8R8A8_UNORM;
      props.pixelFormat.alphaMode = D2D1_ALPHA_MODE_IGNORE;
      props.dpiX = static_cast<float>(dpi == 0 ? 96u : dpi);
      props.dpiY = static_cast<float>(dpi == 0 ? 96u : dpi);
      props.usage = D2D1_RENDER_TARGET_USAGE_GDI_COMPATIBLE;
      props.minLevel = D2D1_FEATURE_LEVEL_DEFAULT;
      HRESULT hr = m_d2dFactory->CreateDCRenderTarget(&props, &m_renderTarget);
      if (FAILED(hr)) return false;
      m_renderTarget->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_CLEARTYPE);
      m_renderDpi = dpi == 0 ? 96u : dpi;
    } else if (m_renderDpi != (dpi == 0 ? 96u : dpi)) {
      m_renderDpi = dpi == 0 ? 96u : dpi;
      m_renderTarget->SetDpi(static_cast<float>(m_renderDpi), static_cast<float>(m_renderDpi));
    }
    const RECT bounds = ResolveDCRenderBounds(hdc, preferredRect);
    HRESULT hr = m_renderTarget->BindDC(hdc, &bounds);
    if (hr == D2DERR_RECREATE_TARGET) {
      if (recreateDepth >= 2) return false;
      ResetRenderTarget();
      return EnsureRenderTargetWithDepth(hdc, preferredRect, dpi, recreateDepth + 1);
    }
    return SUCCEEDED(hr);
  }

  bool EnsureBrush(COLORREF color) {
    if (!m_renderTarget) return false;
    const D2D1_COLOR_F d2dColor = ToD2DColor(color);
    if (m_brush) {
      m_brush->SetColor(d2dColor);
      return true;
    }
    HRESULT hr = m_renderTarget->CreateSolidColorBrush(d2dColor, &m_brush);
    return SUCCEEDED(hr);
  }

  RECT ResolveDCRenderBounds(HDC hdc, const RECT& fallback) const {
    HGDIOBJ bitmap = GetCurrentObject(hdc, OBJ_BITMAP);
    BITMAP bm = {};
    if (bitmap && GetObjectType(bitmap) == OBJ_BITMAP && GetObjectW(bitmap, sizeof(bm), &bm) &&
        bm.bmWidth > 1 && bm.bmHeight > 1 && bm.bmWidth >= fallback.right &&
        bm.bmHeight >= fallback.bottom) {
      return {0, 0, bm.bmWidth, bm.bmHeight};
    }

    RECT clip = {};
    const int clipType = GetClipBox(hdc, &clip);
    if (clipType != ERROR && clipType != NULLREGION && clip.right > clip.left &&
        clip.bottom > clip.top) {
      return clip;
    }

    return {std::min(0L, fallback.left), std::min(0L, fallback.top),
            std::max(1L, fallback.right), std::max(1L, fallback.bottom)};
  }

  void ResetRenderTarget() {
    m_brush.Reset();
    m_renderTarget.Reset();
    m_renderDpi = 0;
  }

  void ClearTextCaches() {
    m_formatCache.clear();
    m_layoutCache.clear();
  }

  static int QuantizeDip(float value) {
    return static_cast<int>(std::lround(value * 64.0f));
  }

  std::wstring MakeFormatKey(const DirectWriteFontSpec& spec, UINT formatFlags,
                             bool customCollection) const {
    std::wstring key = spec.family;
    std::transform(key.begin(), key.end(), key.begin(),
                   [](wchar_t ch) { return static_cast<wchar_t>(towlower(ch)); });
    key += L"|w=" + std::to_wstring(static_cast<int>(spec.weight));
    key += L"|s=" + std::to_wstring(static_cast<int>(spec.style));
    key += L"|t=" + std::to_wstring(static_cast<int>(spec.stretch));
    key += L"|z=" + std::to_wstring(QuantizeDip(spec.sizeDip));
    key += L"|a=" + std::to_wstring(static_cast<int>(ResolveDirectWriteTextAlignment(formatFlags)));
    key += L"|p=" + std::to_wstring(static_cast<int>(ResolveDirectWriteParagraphAlignment(formatFlags)));
    key += L"|r=" + std::to_wstring(static_cast<int>(ResolveDirectWriteWrapping(formatFlags)));
    key += customCollection ? L"|c=1" : L"|c=0";
    key += L"|g=" + std::to_wstring(m_customGeneration);
    return key;
  }

  std::wstring MakeLayoutKey(const std::wstring& formatKey, const std::wstring& text,
                             float widthDip, float heightDip, UINT formatFlags) const {
    std::wstring key = formatKey;
    key += L"|lw=" + std::to_wstring(QuantizeDip(widthDip));
    key += L"|lh=" + std::to_wstring(QuantizeDip(heightDip));
    key += (formatFlags & DT_END_ELLIPSIS) ? L"|e=1|" : L"|e=0|";
    key += text;
    return key;
  }

  void TrimLayoutCache() {
    while (m_layoutCache.size() > kMaxLayoutCacheEntries) {
      auto victim = m_layoutCache.end();
      ULONGLONG oldest = static_cast<ULONGLONG>(-1);
      for (auto it = m_layoutCache.begin(); it != m_layoutCache.end(); ++it) {
        if (it->second.tick < oldest) {
          oldest = it->second.tick;
          victim = it;
        }
      }
      if (victim == m_layoutCache.end()) break;
      m_layoutCache.erase(victim);
      if (m_layoutCache.size() <= kTrimmedLayoutCacheEntries) break;
    }
  }

  Microsoft::WRL::ComPtr<IDWriteFactory> m_dwriteFactory;
  Microsoft::WRL::ComPtr<ID2D1Factory> m_d2dFactory;
  Microsoft::WRL::ComPtr<ID2D1DCRenderTarget> m_renderTarget;
  Microsoft::WRL::ComPtr<ID2D1SolidColorBrush> m_brush;
  Microsoft::WRL::ComPtr<IDWriteFontFallback> m_fontFallback;
  Microsoft::WRL::ComPtr<IDWriteFontCollection1> m_customCollection;
  std::filesystem::path m_customFontPath;
  std::wstring m_customFontFamily;
  bool m_customFamilyAvailable = false;
  UINT m_renderDpi = 0;
  UINT m_customGeneration = 0;
  ULONGLONG m_cacheTick = 0;
  std::unordered_map<std::wstring, bool> m_systemFamilyAvailability;
  std::unordered_map<std::wstring, Microsoft::WRL::ComPtr<IDWriteTextFormat>> m_formatCache;
  std::unordered_map<std::wstring, LayoutCacheEntry> m_layoutCache;
};

DirectTextRenderer*& DirectTextRendererStorage() {
  static DirectTextRenderer* renderer = nullptr;
  return renderer;
}

std::mutex& DirectTextRendererStorageMutex() {
  static std::mutex mutex;
  return mutex;
}

DirectTextRenderer& GetDirectTextRenderer() {
  std::lock_guard<std::mutex> lock(DirectTextRendererStorageMutex());
  DirectTextRenderer*& renderer = DirectTextRendererStorage();
  if (!renderer) renderer = new DirectTextRenderer();
  return *renderer;
}

void ShutdownDirectTextRenderer() {
  std::lock_guard<std::mutex> lock(DirectTextRendererStorageMutex());
  DirectTextRenderer*& renderer = DirectTextRendererStorage();
  delete renderer;
  renderer = nullptr;
}
