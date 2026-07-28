void CSrfTip::EnsureDisplayAttributeAtom() {
  if (m_displayAttrAtom != TF_INVALID_GUIDATOM) return;

  ITfCategoryMgr* categoryMgr = nullptr;
  HRESULT hr = CoCreateInstance(CLSID_TF_CategoryMgr, nullptr, CLSCTX_INPROC_SERVER,
                                IID_ITfCategoryMgr, reinterpret_cast<void**>(&categoryMgr));
  if (FAILED(hr) || !categoryMgr) return;

  (void)categoryMgr->RegisterGUID(GUID_DISPLAY_ATTRIBUTE_SRF_INPUT, &m_displayAttrAtom);
  categoryMgr->Release();
}

void CSrfTip::LoadCompartmentState() {
  m_imeOpen =
      GetCompartmentDWORD(GUID_COMPARTMENT_KEYBOARD_OPENCLOSE,
                          m_config.input.defaultAscii ? 0 : (m_imeOpen ? 1 : 0)) != 0;

  // INPUTMODE_CONVERSION is a thread-manager compartment shared with other
  // input methods.  Reusing its FULLSHAPE bit here can make this IME start with
  // a full-width space and digits merely because the previously active IME left
  // that bit set.  Full shape is a temporary state for this IME, so initialize
  // it from our own configured default and publish it below in
  // SyncCompartmentState().
  m_fullShape = m_config.input.defaultFullShape;
  m_cnPunct = GetCompartmentDWORD(GUID_COMPARTMENT_SRF_PUNCT,
                                  m_config.input.defaultChinesePunct ? 1 : 0) != 0;
  m_fuzzyPinyin =
      GetCompartmentDWORD(GUID_COMPARTMENT_SRF_FUZZY,
                          m_config.input.defaultFuzzyPinyin ? 1 : (m_fuzzyPinyin ? 1 : 0)) != 0;
  m_doublePinyin =
      GetCompartmentDWORD(GUID_COMPARTMENT_SRF_DOUBLE,
                          m_config.input.defaultDoublePinyin ? 1 : (m_doublePinyin ? 1 : 0)) != 0;
  m_traditionalOutput = m_config.input.traditionalOutput;
  ApplyDefaultPunctuationForImeMode();
  ApplyRustModeFlags();
}

void CSrfTip::SyncCompartmentState() {
  SetCompartmentDWORD(GUID_COMPARTMENT_KEYBOARD_OPENCLOSE, m_imeOpen ? 1 : 0);

  DWORD conversion = GetCompartmentDWORD(GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION, 0);
  // In Chinese mode this TIP performs full-shape conversion itself.  Keeping
  // the shared TSF FULLSHAPE bit clear prevents hosts from also converting
  // space and top-row digits to full-width characters.
  const bool publishSystemFullShape = m_fullShape && !m_imeOpen;
  if (publishSystemFullShape) {
    conversion |= TF_CONVERSIONMODE_FULLSHAPE;
  } else {
    conversion &= ~TF_CONVERSIONMODE_FULLSHAPE;
  }
  SetCompartmentDWORD(GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION, conversion);

  SetCompartmentDWORD(GUID_COMPARTMENT_SRF_PUNCT, m_cnPunct ? 1 : 0);
  SetCompartmentDWORD(GUID_COMPARTMENT_SRF_FUZZY, m_fuzzyPinyin ? 1 : 0);
  SetCompartmentDWORD(GUID_COMPARTMENT_SRF_DOUBLE, m_doublePinyin ? 1 : 0);
  ApplyRustModeFlags();
}

void CSrfTip::ApplyRustModeFlags() { SrfTip_SetEngineModeFlags(RustModeFlags()); }

void CSrfTip::ApplyDefaultPunctuationForImeMode() {
  m_cnPunct = m_imeOpen && m_config.input.defaultChinesePunct;
}

DWORD CSrfTip::RustModeFlags() const {
  DWORD flags = 0;
  if (m_fuzzyPinyin) flags |= kRustModeFuzzy;
  if (m_doublePinyin) flags |= kRustModeDouble;
  if (m_config.engine.jianpin) flags |= kRustModeJianpin;
  if (m_config.engine.mixedPinyin) flags |= kRustModeMixedPinyin;
  if (m_config.engine.mixedPinyinAggressive) flags |= kRustModeMixedPinyinAggressive;
  if (m_config.engine.learningSensitivity == L"aggressive") {
    flags |= kRustModeLearningAggressive;
  } else if (m_config.engine.learningSensitivity == L"conservative") {
    flags |= kRustModeLearningConservative;
  }
  if (m_config.engine.vAssist) flags |= kRustModeVAssist;
  if (m_config.engine.uMode) flags |= kRustModeUMode;
  if (m_config.input.dateAutoFormat) flags |= kRustModeDateAutoFormat;
  if (m_config.input.englishWordInput) flags |= kRustModeEnglishWordInput;
  if (m_config.input.symbolToolbox) flags |= kRustModeSymbolToolbox;
  if (m_config.input.emojiInput) flags |= kRustModeEmojiInput;
  if (m_config.clipboard.backgroundEnabled) flags |= kRustModeClipboardBackground;
  if (ShouldSuppressClipboardForPrivacy()) flags |= kRustModeClipboardDisabled;
  if (m_traditionalOutput) flags |= kRustModeTraditionalOutput;
  if (m_userPhraseComposeActive && m_userPhraseComposeValid) {
    flags |= kRustModeUserPhraseComposeActive;
  }
  return flags;
}

DWORD CSrfTip::GetCompartmentDWORD(REFGUID guid, DWORD fallback) const {
  if (!m_pCompartmentMgr) return fallback;

  ITfCompartment* compartment = nullptr;
  HRESULT hr = m_pCompartmentMgr->GetCompartment(guid, &compartment);
  if (FAILED(hr) || !compartment) return fallback;

  VARIANT value;
  VariantInit(&value);
  DWORD out = fallback;
  if (SUCCEEDED(compartment->GetValue(&value)) && value.vt == VT_I4) out = value.lVal;
  VariantClear(&value);
  compartment->Release();
  return out;
}

void CSrfTip::SetCompartmentDWORD(REFGUID guid, DWORD value) {
  if (!m_pCompartmentMgr || m_tid == 0) return;

  ITfCompartment* compartment = nullptr;
  HRESULT hr = m_pCompartmentMgr->GetCompartment(guid, &compartment);
  if (FAILED(hr) || !compartment) return;

  VARIANT var;
  VariantInit(&var);
  var.vt = VT_I4;
  var.lVal = value;
  (void)compartment->SetValue(m_tid, &var);
  compartment->Release();
}

bool CSrfTip::IsConfiguredHotkey(UINT vk, const SrfHotkeyOptions& hotkey) const {
  if (!hotkey.enabled || hotkey.vk == 0 || hotkey.vk != vk) return false;
  const bool ctrlDown = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
  const bool shiftDown = (GetKeyState(VK_SHIFT) & 0x8000) != 0;
  const bool altDown = (GetKeyState(VK_MENU) & 0x8000) != 0;
  const bool needCtrl = (hotkey.modifiers & TF_MOD_CONTROL) != 0;
  const bool needShift = (hotkey.modifiers & TF_MOD_SHIFT) != 0;
  const bool needAlt = (hotkey.modifiers & TF_MOD_ALT) != 0;
  return ctrlDown == needCtrl && shiftDown == needShift && altDown == needAlt;
}

void CSrfTip::ToggleImeOpen() {
  m_imeOpen = !m_imeOpen;
  if (!m_imeOpen && m_pComposition) RequestCancelCompositionOnFocusLoss();
  ApplyDefaultPunctuationForImeMode();
  // Scheme-A: always persist global ASCII state.
  SaveGlobalAsciiState(!m_imeOpen);
  SyncCompartmentState();
  SyncStatusModel();
  RebuildContextModel();
  RedrawCandidateUi();
  if (m_config.ShouldShowNotification(SrfNotificationKind::Ime)) {
    // 与设置页说明一致：轻按 Shift 切换；通知用语使用中文。
    ShowNotification(SrfNotificationKind::Ime, m_imeOpen ? L"\u4e2d\u6587" : L"\u82f1\u6587");
  }
}

void CSrfTip::ToggleFullShape() {
  m_fullShape = !m_fullShape;
  SyncCompartmentState();
  SyncStatusModel();
  if (m_config.ShouldShowNotification(SrfNotificationKind::FullShape)) {
    ShowNotification(SrfNotificationKind::FullShape, m_fullShape ? L"Full shape on"
                                                                 : L"Full shape off");
  }
}

void CSrfTip::ToggleChinesePunctuation() {
  m_cnPunct = !m_cnPunct;
  SyncCompartmentState();
  SyncStatusModel();
  if (m_config.ShouldShowNotification(SrfNotificationKind::Punctuation)) {
    ShowNotification(SrfNotificationKind::Punctuation,
                     m_cnPunct ? L"Chinese punctuation on" : L"Chinese punctuation off");
  }
}

void CSrfTip::ToggleFuzzyPinyin() {
  m_fuzzyPinyin = !m_fuzzyPinyin;
  SyncCompartmentState();
  SyncStatusModel();
  if (!m_reading.empty()) {
    RefreshCandidatesAsync();
    RedrawCandidateUi();
  }
  if (m_config.ShouldShowNotification(SrfNotificationKind::Fuzzy)) {
    ShowNotification(SrfNotificationKind::Fuzzy,
                     m_fuzzyPinyin ? L"Fuzzy pinyin on" : L"Fuzzy pinyin off");
  }
}

void CSrfTip::ToggleDoublePinyin() {
  m_doublePinyin = !m_doublePinyin;
  SyncCompartmentState();
  SyncStatusModel();
  if (!m_reading.empty()) {
    RefreshCandidatesAsync();
    RedrawCandidateUi();
  }
  if (m_config.ShouldShowNotification(SrfNotificationKind::DoublePinyin)) {
    ShowNotification(SrfNotificationKind::DoublePinyin,
                     m_doublePinyin ? L"Double pinyin on" : L"Double pinyin off");
  }
}

void CSrfTip::ToggleTraditionalOutput() {
  m_traditionalOutput = !m_traditionalOutput;
  ApplyRustModeFlags();
  SyncStatusModel();
  if (!m_reading.empty()) {
    RefreshCandidatesAsync();
    RedrawCandidateUi();
  }
  if (m_config.ShouldShowNotification(SrfNotificationKind::Ime)) {
    ShowNotification(SrfNotificationKind::Ime,
                     m_traditionalOutput ? L"\u7e41\u9ad4\u8f38\u51fa"
                                         : L"\u7b80\u4f53\u8f93\u51fa");
  }
}

void CSrfTip::ToggleManualGameCompat() {
  m_manualGameCompatActive = !m_manualGameCompatActive;
  SyncStatusModel();
  RebuildContextModel();
  RedrawCandidateUi();
  if (m_config.ShouldShowNotification(SrfNotificationKind::AppOptions)) {
    ShowNotification(SrfNotificationKind::AppOptions,
                     m_manualGameCompatActive ? L"\u6e38\u620f\u517c\u5bb9\u5f00"
                                              : L"\u6e38\u620f\u517c\u5bb9\u5173");
  }
}

void CSrfTip::ToggleManualAsciiMode(TfEditCookie ec) {
  m_manualAsciiModeActive = !m_manualAsciiModeActive;
  if (m_manualAsciiModeActive) {
    if (m_candidateUi) m_candidateUi->End();
    if (m_pComposition) {
      CancelCompositionEdit(ec);
    } else {
      ReleaseCompositionState();
    }
  }
  SyncStatusModel();
  RebuildContextModel();
  RedrawCandidateUi();
  if (m_config.ShouldShowNotification(SrfNotificationKind::Ime)) {
    ShowNotification(SrfNotificationKind::Ime,
                     m_manualAsciiModeActive ? L"\u4e34\u65f6\u82f1\u6587\u5f00"
                                             : L"\u4e34\u65f6\u82f1\u6587\u5173");
  }
}

void CSrfTip::LearnCommittedText(const std::wstring& committedText) {
  if (m_sensitiveInputActive || ShouldForceAsciiForCompatibility() ||
      ShouldSuppressLearningForPrivacy()) {
    return;
  }
  if (!m_reading.empty() && !committedText.empty()) {
    SrfTip_LearnCommit(m_reading, committedText);
  }
}

HRESULT CSrfTip::RequestCommitCandidate(size_t idx) {
  ITfContext* context = m_pCompositionContext ? m_pCompositionContext : m_pFocusContext;
  if (!context || m_tid == 0) return E_FAIL;
  if (m_candidatesReading != m_reading) return S_OK;

  const std::wstring snapshotReading = m_reading;
  const std::wstring snapshotCommitted =
      idx < m_candidates.size() ? m_candidates[idx] : std::wstring();
  const std::wstring snapshotMeta =
      idx < m_candidateMeta.size() ? m_candidateMeta[idx] : std::wstring();
  std::vector<std::wstring> snapshotSkippedCandidates;
  if (idx < m_candidates.size()) {
    const UINT pageStart = CandidatePageStart(m_candPage);
    const UINT pageEndExclusive = CandidatePageEndExclusive(m_candPage);
    const size_t weakStart = std::max(static_cast<size_t>(pageStart), idx + 1);
    const size_t weakEnd =
        std::min(static_cast<size_t>(pageEndExclusive), m_candidates.size());
    snapshotSkippedCandidates.reserve(idx + (weakEnd > weakStart ? weakEnd - weakStart : 0));
    for (size_t i = 0; i < idx; ++i) {
      snapshotSkippedCandidates.push_back(m_candidates[i]);
    }
    for (size_t i = weakStart; i < weakEnd; ++i) {
      snapshotSkippedCandidates.push_back(m_candidates[i]);
    }
  }
  if (!snapshotMeta.empty() && SplitCandidateMeta(snapshotMeta).prefixPlaceholder) {
    std::wstring line = L"idx=" + std::to_wstring(idx);
    line += L", reading=";
    line += ShortenForLog(m_reading, 24);
    SrfTsfDiagnosticLog(L"commit-candidate.skip-placeholder", line.c_str());
    return S_OK;
  }

  CEditSessionCommitCandidate* edit =
      new (std::nothrow) CEditSessionCommitCandidate(this, context, idx, snapshotReading,
                                                     snapshotCommitted, snapshotMeta,
                                                     std::move(snapshotSkippedCandidates));
  if (!edit) return E_OUTOFMEMORY;

  HRESULT hrSession = E_FAIL;
  const ULONGLONG requestStart = GetTickCount64();
  HRESULT hr =
      context->RequestEditSession(m_tid, edit, TF_ES_SYNC | TF_ES_READWRITE, &hrSession);
  if (FAILED(hr) || hrSession == TS_E_SYNCHRONOUS) {
    hrSession = E_FAIL;
    hr = context->RequestEditSession(m_tid, edit, TF_ES_ASYNC | TF_ES_READWRITE, &hrSession);
  }
  const ULONGLONG requestElapsed = GetTickCount64() - requestStart;
  edit->Release();
  std::wstring line = L"idx=" + std::to_wstring(idx);
  line += L", wait_ms=";
  line += std::to_wstring(requestElapsed);
  wchar_t hrBuf[16] = {};
  line += L", hr=0x";
  swprintf_s(hrBuf, L"%08lX", static_cast<unsigned long>(hr));
  line += hrBuf;
  line += L", session=0x";
  swprintf_s(hrBuf, L"%08lX", static_cast<unsigned long>(hrSession));
  line += hrBuf;
  line += L", reading_len=";
  line += std::to_wstring(snapshotReading.size());
  line += L", committed_len=";
  line += std::to_wstring(snapshotCommitted.size());
  SrfTsfDiagnosticLog(L"commit-candidate.request-edit-session", line.c_str());
  if (FAILED(hr)) return hr;
  return hrSession;
}

HRESULT CSrfTip::RequestCommitReadingText() {
  ITfContext* context = m_pCompositionContext ? m_pCompositionContext : m_pFocusContext;
  if (!context || m_tid == 0) return E_FAIL;

  CEditSessionCommitReadingText* edit = new (std::nothrow) CEditSessionCommitReadingText(this);
  if (!edit) return E_OUTOFMEMORY;

  HRESULT hrSession = E_FAIL;
  const HRESULT hr =
      context->RequestEditSession(m_tid, edit, TF_ES_ASYNC | TF_ES_READWRITE, &hrSession);
  edit->Release();
  if (FAILED(hr)) return hr;
  return hrSession;
}

std::wstring CSrfTip::TranslateDirectKey(UINT vk, LPARAM lParam, bool shiftDown) const {
  if (vk == VK_SPACE) return L" ";

  std::array<BYTE, 256> keyboardState = {};
  (void)GetKeyboardState(keyboardState.data());
  auto syncToggleOrModifier = [&](int key) {
    const SHORT state = GetKeyState(key);
    keyboardState[key] =
        static_cast<BYTE>(((state & 0x8000) != 0 ? 0x80 : 0x00) | ((state & 0x0001) != 0 ? 0x01 : 0x00));
  };
  syncToggleOrModifier(VK_CONTROL);
  syncToggleOrModifier(VK_MENU);
  syncToggleOrModifier(VK_CAPITAL);
  syncToggleOrModifier(VK_NUMLOCK);
  keyboardState[VK_SHIFT] =
      static_cast<BYTE>((keyboardState[VK_SHIFT] & 0x01) | (shiftDown ? 0x80 : 0x00));
  keyboardState[VK_LSHIFT] =
      static_cast<BYTE>((keyboardState[VK_LSHIFT] & 0x01) | (shiftDown ? 0x80 : 0x00));
  keyboardState[VK_RSHIFT] =
      static_cast<BYTE>((keyboardState[VK_RSHIFT] & 0x01) | (shiftDown ? 0x80 : 0x00));
  const bool capsLockOn = (keyboardState[VK_CAPITAL] & 0x01) != 0;

  const UINT scanCode =
      HIWORD(static_cast<DWORD>(lParam)) & 0xff ? (HIWORD(static_cast<DWORD>(lParam)) & 0xff)
                                                : MapVirtualKeyW(vk, MAPVK_VK_TO_VSC);
  std::array<wchar_t, 8> buffer = {};
  const int rc = ToUnicodeEx(vk, scanCode, keyboardState.data(), buffer.data(),
                             static_cast<int>(buffer.size()), 0, GetKeyboardLayout(0));
  if (rc < 0) {
    std::array<BYTE, 256> emptyState = {};
    (void)ToUnicodeEx(vk, scanCode, emptyState.data(), buffer.data(),
                      static_cast<int>(buffer.size()), 0, GetKeyboardLayout(0));
    return FallbackPrintableText(vk, shiftDown, capsLockOn);
  }
  if (rc <= 0) return FallbackPrintableText(vk, shiftDown, capsLockOn);
  return std::wstring(buffer.data(), rc);
}

namespace {

bool SrfBuildCompletedPunctuationPair(wchar_t ch, std::wstring* pair) {
  if (!pair) return false;

  switch (ch) {
    case L'(':
      pair->assign(L"()");
      return true;
    case L'[':
      pair->assign(L"[]");
      return true;
    case L'{':
      pair->assign(L"{}");
      return true;
    case L'<':
      pair->assign(L"<>");
      return true;
    case L'"':
      pair->assign(L"\"\"");
      return true;
    case L'\'':
      pair->assign(L"''");
      return true;
    case L'\uff08':
      pair->assign(L"\uff08\uff09");
      return true;
    case L'\u3010':
      pair->assign(L"\u3010\u3011");
      return true;
    case L'\uff3b':
      pair->assign(L"\uff3b\uff3d");
      return true;
    case L'\uff5b':
      pair->assign(L"\uff5b\uff5d");
      return true;
    case L'\u300a':
      pair->assign(L"\u300a\u300b");
      return true;
    case L'\uff1c':
      pair->assign(L"\uff1c\uff1e");
      return true;
    case L'\u201c':
      pair->assign(L"\u201c\u201d");
      return true;
    case L'\u2018':
      pair->assign(L"\u2018\u2019");
      return true;
    case L'\uff02':
      pair->assign(L"\uff02\uff02");
      return true;
    case L'\uff07':
      pair->assign(L"\uff07\uff07");
      return true;
    case L'\u3008':
      pair->assign(L"\u3008\u3009");
      return true;
    case L'\u300c':
      pair->assign(L"\u300c\u300d");
      return true;
    case L'\u300e':
      pair->assign(L"\u300e\u300f");
      return true;
    case L'\u3014':
      pair->assign(L"\u3014\u3015");
      return true;
    case L'\u3016':
      pair->assign(L"\u3016\u3017");
      return true;
    default:
      return false;
  }
}

}  // namespace

std::wstring CSrfTip::ConvertDirectText(std::wstring text) {
  std::wstring converted;
  converted.reserve(text.size() * 2);

  for (wchar_t ch : text) {
    std::wstring replacement;
    if (m_cnPunct) {
      switch (ch) {
        case L',':
          replacement = L"，";
          break;
        case L'.':
          replacement = L"。";
          break;
        case L'?':
          replacement = L"？";
          break;
        case L'!':
          replacement = L"！";
          break;
        case L';':
          replacement = L"；";
          break;
        case L':':
          replacement = L"：";
          break;
        case L'(':
          replacement = L"（";
          break;
        case L')':
          replacement = L"）";
          break;
        case L'[':
          replacement = L"【";
          break;
        case L']':
          replacement = L"】";
          break;
        case L'<':
          replacement = L"《";
          break;
        case L'>':
          replacement = L"》";
          break;
        case L'\\':
          replacement = L"、";
          break;
        case L'/':
          replacement = L"、";
          break;
        case L'"':
          replacement = m_nextDoubleQuoteOpen ? L"“" : L"”";
          m_nextDoubleQuoteOpen = !m_nextDoubleQuoteOpen;
          break;
        case L'\'':
          replacement = m_nextSingleQuoteOpen ? L"‘" : L"’";
          m_nextSingleQuoteOpen = !m_nextSingleQuoteOpen;
          break;
        default:
          break;
      }
    }

    if (m_cnPunct && !m_config.input.symbolFullwidth) {
      switch (ch) {
        case L'(':
        case L')':
        case L'[':
        case L']':
        case L'<':
        case L'>':
        case L'\\':
        case L'/':
          replacement.clear();
          break;
        default:
          break;
      }
    }

    if (m_cnPunct && !m_config.input.curlyPunct && ch == L'"') {
      replacement = L"\uff02";
    } else if (m_cnPunct && !m_config.input.curlyPunct && ch == L'\'') {
      replacement = L"\uff07";
    }

    if (replacement.empty() && m_config.input.symbolFullwidth && IsAsciiPunctuationChar(ch)) {
      replacement.push_back(static_cast<wchar_t>(0xff01 + (ch - 0x21)));
    }

    if (replacement.empty() && m_config.input.numberFullwidth && ch >= L'0' && ch <= L'9') {
      replacement.push_back(static_cast<wchar_t>(0xff10 + (ch - L'0')));
    }

    if (replacement.empty() && m_fullShape) {
      if (ch >= 0x21 && ch <= 0x7e && !(ch >= L'0' && ch <= L'9')) {
        replacement.push_back(static_cast<wchar_t>(0xff01 + (ch - 0x21)));
      }
    }

    if (replacement.empty()) replacement.push_back(ch);
    converted += replacement;
  }

  return converted;
}

std::wstring CSrfTip::ConvertDirectTextWithCompletion(std::wstring text, LONG* cursorOffset) {
  if (cursorOffset) *cursorOffset = -1;

  if (m_config.input.autoPairPunct && text.size() == 1) {
    std::wstring pair;
    const wchar_t raw = text[0];
    if (raw == L'"') {
      if (m_cnPunct && m_config.input.curlyPunct) {
        pair.assign(L"\u201c\u201d");
      } else if (m_cnPunct || m_config.input.symbolFullwidth || m_fullShape) {
        pair.assign(L"\uff02\uff02");
      } else {
        pair.assign(L"\"\"");
      }
      if (cursorOffset) *cursorOffset = 1;
      return pair;
    }
    if (raw == L'\'') {
      if (m_cnPunct && m_config.input.curlyPunct) {
        pair.assign(L"\u2018\u2019");
      } else if (m_cnPunct || m_config.input.symbolFullwidth || m_fullShape) {
        pair.assign(L"\uff07\uff07");
      } else {
        pair.assign(L"''");
      }
      if (cursorOffset) *cursorOffset = 1;
      return pair;
    }
  }

  std::wstring converted = ConvertDirectText(std::move(text));
  if (m_config.input.autoPairPunct && converted.size() == 1) {
    std::wstring pair;
    if (SrfBuildCompletedPunctuationPair(converted[0], &pair)) {
      if (cursorOffset) *cursorOffset = 1;
      return pair;
    }
  }

  return converted;
}

bool CSrfTip::ShouldHandleDirectKey(UINT vk) const {
  if (HasCtrlOrAltDown()) return false;
  return IsLetterVk(vk) || IsDigitVk(vk) || IsNumpadPrintableVk(vk) || IsOemPrintableVk(vk) ||
         vk == VK_SPACE;
}

bool CSrfTip::ShouldUseTemporaryEnglish(UINT vk, bool shiftDown) const {
  const bool capsLockOn = (GetKeyState(VK_CAPITAL) & 0x0001) != 0;
  // CapsLock on keeps letter keys in direct English while Chinese mode is idle.
  // Shift with number-row symbols or OEM punctuation is optional temporary ASCII input.
  const bool temporaryLetter = IsLetterVk(vk) && (shiftDown || capsLockOn);
  const bool temporarySymbol = m_config.input.shiftSymbolTemporaryAscii && shiftDown &&
                               (IsDigitVk(vk) || IsOemPrintableVk(vk));
  return m_imeOpen && m_reading.empty() && !HasCtrlOrAltDown() &&
         (temporaryLetter || temporarySymbol);
}

std::wstring CSrfTip::BuildCompositionDisplay() const { return m_reading; }

void CSrfTip::LoadConfiguration() {
  m_config = LoadSrfConfig();
  m_loadedConfigVersion = GetSrfConfigVersion();
  SrfTip_SetRetryOnFailureEnabled(m_config.engine.retryOnFailure);
}

bool CSrfTip::ReloadConfigurationIfChanged() {
  const uint64_t version = GetSrfConfigVersion();
  if (version == 0 || version == m_loadedConfigVersion) {
    return false;
  }

  LoadConfiguration();
  return true;
}

bool CSrfTip::HasConfigurationChanged() const {
  const uint64_t version = GetSrfConfigVersion();
  return version != 0 && version != m_loadedConfigVersion;
}

void CSrfTip::ApplyRuntimeConfigReload() {
  if (!ReloadConfigurationIfChanged()) {
    m_configReloadPending = false;
    return;
  }
  m_configReloadPending = false;
  UnregisterPreservedKeys();
  (void)RegisterPreservedKeys();
  m_uiStyle = ResolveUiStyleForApp(m_activeAppName);
  InvalidateCandidatePageLayoutCache();
  InvalidateHotPathStateCache();
  RefreshCompatibilityState();
  ApplyRustModeFlags();
  ClampCandidateState();
  RebuildContextModel();
  SyncStatusModel();
  SrfTsfDiagnosticLog(L"config-reload.apply", L"runtime configuration reloaded");
}

void CSrfTip::ApplyPendingRuntimeConfigIfSafe() {
  if (!m_configReloadPending) return;
  if (!m_reading.empty()) return;
  ApplyRuntimeConfigReload();
}

SrfUIStyle CSrfTip::ResolveUiStyleForApp(const std::wstring& appName) const {
  SrfUIStyle style = m_config.style;
  if (const SrfAppOptions* matchedOptions = FindAppOptions(m_config, appName)) {
    const SrfAppOptions& options = *matchedOptions;
    if (options.hasInlinePreedit) style.inlinePreedit = options.inlinePreedit;
    if (options.hasEnhancedPosition) style.enhancedPosition = options.enhancedPosition;
    if (options.hasCandidateTopmost) style.candidateTopmost = options.candidateTopmost;
    if (options.hasOverlayScale) {
      style.candidateScalePercent = std::clamp(options.overlayScalePercent, 50u, 200u);
    }
    if (options.hasOverlayAnchor) style.candidateOverlayAnchor = options.overlayAnchor;
    if (options.hasGameProfile && options.gameCompactProfile) {
      style.candidateHorizontal = true;
      style.candidateHorizontalCompact = true;
      style.candidatePageSize = std::min(style.candidatePageSize, 5u);
      style.candidateHorizontalCount = std::min(style.candidateHorizontalCount, 5u);
      style.candidateTopmost = true;
      style.candidateOpacity = 100;
      style.candidateMaterial = SrfCandidateMaterial::Solid;
      style.candidateDensity = SrfCandidateDensity::Compact;
      style.candidateLayoutVariant = SrfCandidateLayoutVariant::Compact;
      style.candidateLeftClick = false;
      style.candidateRightClick = false;
      style.showCandidateReading = false;
      style.showCandidateScore = false;
      style.showCandidateSource = false;
      style.showModeInCandidateHeader = false;
    }
  }
  return style;
}

SrfFocusPolicy CSrfTip::EffectiveFocusPolicy() const {
  const SrfAppOptions* options = FindAppOptions(m_config, m_activeAppName);
  if (options && options->hasFocusPolicy) {
    return options->focusPolicy;
  }
  if (IsBuiltinStrictFocusProcessName(m_activeAppName)) return SrfFocusPolicy::Strict;
  return SrfFocusPolicy::Normal;
}

const wchar_t* CSrfTip::EffectiveFocusPolicyName() const {
  switch (EffectiveFocusPolicy()) {
    case SrfFocusPolicy::Strict:
      return L"strict";
    case SrfFocusPolicy::Window:
      return L"window";
    case SrfFocusPolicy::Normal:
    default:
      return L"normal";
  }
}

void CSrfTip::InvalidateHotPathStateCache() {
  m_lastKeyHotPathRefreshTick = 0;
  m_lastCompatibilityRefreshTick = 0;
}

void CSrfTip::RefreshRuntimeConfig() {
  if (!HasConfigurationChanged()) {
    ApplyPendingRuntimeConfigIfSafe();
    return;
  }
  if (!m_reading.empty()) {
    if (!m_configReloadPending) {
      SrfTsfDiagnosticLog(L"config-reload.defer", L"composition active; deferring reload");
    }
    m_configReloadPending = true;
    return;
  }
  ApplyRuntimeConfigReload();
}

void CSrfTip::RefreshKeyHotPathState() {
  const ULONGLONG now = GetTickCount64();
  if (m_lastKeyHotPathRefreshTick != 0 &&
      now - m_lastKeyHotPathRefreshTick < kKeyHotPathRefreshMs) {
    ApplyPendingRuntimeConfigIfSafe();
    SyncStatusModel();
    return;
  }

  RefreshRuntimeConfig();
  RefreshCompatibilityState();
  m_lastCompatibilityRefreshTick = now;
  ApplyGlobalAsciiStateFromRegistry();
  SyncStatusModel();
  m_lastKeyHotPathRefreshTick = now;
}

void CSrfTip::RefreshCompatibilityStateThrottled(bool force) {
  const ULONGLONG now = GetTickCount64();
  if (!force && m_lastCompatibilityRefreshTick != 0 &&
      now - m_lastCompatibilityRefreshTick < kCompatibilityRefreshMs) {
    return;
  }
  RefreshCompatibilityState();
  m_lastCompatibilityRefreshTick = now;
}

void CSrfTip::RefreshCompatibilityState() {
  const SrfUIStyle previousEffectiveStyle = EffectiveCandidateUiStyle();

  HWND hwnd = GetForegroundWindow();
  HWND root = hwnd ? GetAncestor(hwnd, GA_ROOT) : nullptr;
  if (root && IsWindowVisible(root)) hwnd = root;

  const bool wasSensitiveInputActive = m_sensitiveInputActive;
  m_sensitiveInputActive = SrfIsSensitiveInputContext(m_activeAppName);
  if (!wasSensitiveInputActive && m_sensitiveInputActive) {
    SrfTip_ClearLookupCache();
    ReleaseCompositionState();
    SrfTsfDiagnosticLog(L"sensitive-context.enter", L"status=cleared sensitive=1");
  }

  std::wstring className = WindowClassName(hwnd);
  const bool rawConfiguredGameCompat = IsConfiguredGameProcessName(m_config, m_activeAppName);
  const bool rawBuiltinGameCompat =
      m_config.compatibility.builtinGameList &&
      (IsBuiltinGameProcessName(m_activeAppName) || IsBuiltinGameWindowClass(className));
  const bool rawGameCompat = rawConfiguredGameCompat || rawBuiltinGameCompat;

  const ULONGLONG now = GetTickCount64();
  const bool rawFullscreen =
      m_config.compatibility.fullscreenDetection && IsFullscreenForegroundWindow(hwnd);
  if (!rawFullscreen) {
    m_fullscreenCompatCandidateHwnd = nullptr;
    m_fullscreenCompatCandidateSince = 0;
  } else if (m_fullscreenCompatCandidateHwnd != hwnd) {
    m_fullscreenCompatCandidateHwnd = hwnd;
    m_fullscreenCompatCandidateSince = now;
  }
  const bool stableFullscreen =
      rawFullscreen &&
      (m_fullscreenCompatActive ||
       (m_fullscreenCompatCandidateSince != 0 &&
        now - m_fullscreenCompatCandidateSince >= kFullscreenCompatStableMs));

  const bool rawCompat = rawGameCompat || stableFullscreen;
  if (rawCompat) {
    m_compatLastRawHitTick = now;
    m_gameCompatActive = rawGameCompat;
    m_configuredGameCompatActive = rawConfiguredGameCompat;
    m_builtinGameCompatActive = rawBuiltinGameCompat;
    m_fullscreenCompatActive = stableFullscreen;
  } else if (m_compatLastRawHitTick != 0 &&
             now - m_compatLastRawHitTick < kCompatibilityReleaseDebounceMs) {
    // Keep the previous protection briefly while launchers hand off to the real game window.
  } else {
    m_gameCompatActive = false;
    m_configuredGameCompatActive = false;
    m_builtinGameCompatActive = false;
    m_fullscreenCompatActive = false;
  }
  if (m_runtimeHideUiFallbackActive &&
      (m_activeAppName.empty() ||
       !WildcardMatchNoCase(m_runtimeHideUiFallbackAppName, m_activeAppName))) {
    m_runtimeHideUiFallbackActive = false;
    m_runtimeHideUiFallbackAppName.clear();
  }
  if (m_runtimeAsciiFallbackActive &&
      (m_activeAppName.empty() ||
       !WildcardMatchNoCase(m_runtimeAsciiFallbackAppName, m_activeAppName))) {
    m_runtimeAsciiFallbackActive = false;
    m_runtimeAsciiFallbackAppName.clear();
  }

  const SrfFullscreenPolicy policy = EffectiveCompatibilityPolicy();
  const bool compatibilityHidesUi =
      policy == SrfFullscreenPolicy::Ascii || policy == SrfFullscreenPolicy::HideUi;
  m_uiStyle.candidateTopmost =
      ResolveUiStyleForApp(m_activeAppName).candidateTopmost && !compatibilityHidesUi;

  if (ShouldSuppressCandidatesForPrivacy() && !m_candidates.empty()) {
    m_candidates.clear();
    m_candidateMeta.clear();
    m_candidatesReading.clear();
    SetCandidateViewState(SrfCandidateViewState::Empty, L"privacy-refresh");
    InvalidateCandidatePageLayoutCache();
    ClampCandidateState();
    RebuildContextModel();
  }

  const SrfUIStyle currentEffectiveStyle = EffectiveCandidateUiStyle();
  if (!m_candidates.empty() &&
      !CandidateWindowLayoutStyleEquals(previousEffectiveStyle, currentEffectiveStyle)) {
    InvalidateCandidatePageLayoutCache();
    ClampCandidateState();
    RebuildContextModel();
  }
}

SrfFullscreenPolicy CSrfTip::EffectiveCompatibilityPolicy() const {
  if (m_config.privacy.enabled) return SrfFullscreenPolicy::Ascii;
  if (m_manualAsciiModeActive) return SrfFullscreenPolicy::Ascii;
  if (m_manualGameCompatActive) return m_config.compatibility.fullscreenPolicy;
  if (m_sensitiveInputActive) return SrfFullscreenPolicy::Ascii;
  if (m_runtimeAsciiFallbackActive) return SrfFullscreenPolicy::Ascii;
  if (m_runtimeHideUiFallbackActive) return SrfFullscreenPolicy::HideUi;
  if (IsBuiltinAsciiOnlyProcessName(m_activeAppName)) return SrfFullscreenPolicy::Ascii;
  if (const SrfAppOptions* matchedOptions = FindAppOptions(m_config, m_activeAppName)) {
    const SrfAppOptions& options = *matchedOptions;
    if (options.hasHideUi && options.hideUi) return SrfFullscreenPolicy::HideUi;
    if (options.hasAsciiMode && options.asciiMode) return SrfFullscreenPolicy::Ascii;
    if ((options.hasAsciiMode && !options.asciiMode) ||
        (options.hasHideUi && !options.hideUi)) {
      return SrfFullscreenPolicy::ShowUi;
    }
  }
  if (m_configuredGameCompatActive || m_builtinGameCompatActive || m_gameCompatActive) {
    return m_config.compatibility.fullscreenPolicy;
  }
  if (!m_fullscreenCompatActive) return SrfFullscreenPolicy::Off;
  return m_config.compatibility.fullscreenPolicy;
}

bool CSrfTip::ShouldHideUiForCompatibility() const {
  if (ShouldSuppressCandidatesForPrivacy()) return true;
  const SrfFullscreenPolicy policy = EffectiveCompatibilityPolicy();
  return policy == SrfFullscreenPolicy::Ascii || policy == SrfFullscreenPolicy::HideUi;
}

const wchar_t* CSrfTip::EffectiveCompatibilityPolicyName() const {
  switch (EffectiveCompatibilityPolicy()) {
    case SrfFullscreenPolicy::Ascii:
      return L"ascii";
    case SrfFullscreenPolicy::HideUi:
      return L"hide_ui";
    case SrfFullscreenPolicy::ShowUi:
      return L"show_ui";
    case SrfFullscreenPolicy::Off:
    default:
      return L"off";
  }
}

SrfOverlayBackend CSrfTip::EffectiveCandidateOverlayBackend() const {
  if (const SrfAppOptions* options = FindAppOptions(m_config, m_activeAppName)) {
    if (options->hasOverlayBackend) return options->overlayBackend;
  }
  return SrfOverlayBackend::Auto;
}

bool CSrfTip::ShouldUseExternalCandidateOverlay() const {
  return ShouldUseExternalCandidateOverlayBackend(
      EffectiveCandidateOverlayBackend(), FullscreenCandidateOverlayActive(),
      m_uiLessMode, ShouldHideUiForCompatibility());
}

SrfCommitTransport CSrfTip::EffectiveCommitTransport() const {
  SrfCommitTransport requested = m_config.compatibility.commitTransport;
  bool appGameProfile = false;
  const SrfAppOptions* options = FindAppOptions(m_config, m_activeAppName);
  if (options && options->hasCommitTransport) {
    requested = options->commitTransport;
  }
  if (options && options->hasGameProfile && options->gameCompactProfile) {
    appGameProfile = true;
  }
  if (requested == SrfCommitTransport::Auto) {
    const bool compatibilityActive =
        appGameProfile || m_gameCompatActive || m_configuredGameCompatActive ||
        m_builtinGameCompatActive || m_fullscreenCompatActive || m_manualGameCompatActive;
    return compatibilityActive ? SrfCommitTransport::UnicodeSendInput : SrfCommitTransport::Tsf;
  }
  return requested;
}

const wchar_t* CSrfTip::EffectiveCommitTransportName() const {
  switch (EffectiveCommitTransport()) {
    case SrfCommitTransport::ClipboardPaste:
      return L"clipboard_paste";
    case SrfCommitTransport::UnicodeSendInput:
      return L"unicode_sendinput";
    case SrfCommitTransport::Auto:
      return L"auto";
    case SrfCommitTransport::Tsf:
    default:
      return L"tsf";
  }
}

bool CSrfTip::ShouldForceAsciiForCompatibility() const {
  return EffectiveCompatibilityPolicy() == SrfFullscreenPolicy::Ascii;
}

bool CSrfTip::ShouldUseBlindCommitForCompatibility() const {
  if (ShouldSuppressCandidatesForPrivacy()) return false;
  return EffectiveCompatibilityPolicy() == SrfFullscreenPolicy::HideUi;
}

bool CSrfTip::ShouldSuppressLearningForPrivacy() const {
  return m_config.privacy.enabled ||
         MatchesConfiguredProcessList(m_config.privacy.neverLearnProcessList, m_activeAppName);
}

bool CSrfTip::ShouldSuppressClipboardForPrivacy() const {
  return m_config.privacy.enabled ||
         MatchesConfiguredProcessList(m_config.privacy.neverClipboardProcessList, m_activeAppName);
}

bool CSrfTip::ShouldSuppressCandidatesForPrivacy() const {
  return m_config.privacy.enabled ||
         MatchesConfiguredProcessList(m_config.privacy.neverCandidateProcessList, m_activeAppName);
}

void CSrfTip::RecordCompatibilityUiFallback(const wchar_t* reason, HRESULT hr) {
  if (!m_config.compatibility.autoSuggestAppOptions) return;
  if (EffectiveCompatibilityPolicy() != SrfFullscreenPolicy::ShowUi) return;

  std::wstring appName = m_activeAppName;
  if (appName.empty()) appName = FocusedProcessName();
  if (appName.empty()) appName = L"(unknown)";

  m_runtimeHideUiFallbackActive = true;
  m_runtimeHideUiFallbackAppName = appName;

  wchar_t hrText[32] = {};
  swprintf_s(hrText, L"0x%08lX", static_cast<unsigned long>(hr));
  const std::wstring key =
      appName + L"|ui|" + (reason ? reason : L"CandidateUI") + L"|" + hrText;
  if (key == m_lastCompatibilityLogKey) return;
  m_lastCompatibilityLogKey = key;

  std::wstring line = L"Compatibility fallback: app=" + appName;
  line += L", api=";
  line += reason ? reason : L"CandidateUI";
  line += L", hr=";
  line += hrText;
  line += L", runtime=hide_ui, suggested_ini=[app:";
  line += appName;
  line += L"] hide_ui=1";
  AppendUtf8FileLine(LocalDataDir() / L"compatibility.log", line);

  if (SrfTsfDebugTraceEnabled()) {
    SrfTsfDebugLog(line.c_str());
  }
}

void CSrfTip::RecordCompatibilityFallback(const wchar_t* apiName, HRESULT hr) {
  if (!m_config.compatibility.autoSuggestAppOptions) return;

  std::wstring appName = m_activeAppName;
  if (appName.empty()) appName = FocusedProcessName();
  if (appName.empty()) appName = L"(unknown)";

  m_runtimeAsciiFallbackActive = true;
  m_runtimeAsciiFallbackAppName = appName;

  wchar_t hrText[32] = {};
  swprintf_s(hrText, L"0x%08lX", static_cast<unsigned long>(hr));
  const std::wstring key = appName + L"|" + (apiName ? apiName : L"TSF") + L"|" + hrText;
  if (key == m_lastCompatibilityLogKey) return;
  m_lastCompatibilityLogKey = key;

  std::wstring line = L"Compatibility fallback: app=" + appName;
  line += L", api=";
  line += apiName ? apiName : L"TSF";
  line += L", hr=";
  line += hrText;
  line += L", runtime=ascii, suggested_ini=[app:";
  line += appName;
  line += L"] ascii_mode=1";
  AppendUtf8FileLine(LocalDataDir() / L"compatibility.log", line);

  if (SrfTsfDebugTraceEnabled()) {
    SrfTsfDebugLog(line.c_str());
  }
}

void CSrfTip::SyncStatusModel() {
  m_status.Reset();
  m_status.asciiMode = !m_imeOpen || ShouldForceAsciiForCompatibility();
  m_status.disabled = ShouldForceAsciiForCompatibility();
  m_status.composing = !m_reading.empty();
  m_status.fullShape = m_fullShape;
  m_status.chinesePunctuation = m_cnPunct;
  m_status.fuzzyPinyin = m_fuzzyPinyin;
  m_status.doublePinyin = m_doublePinyin;
  m_status.appName = m_activeAppName;
  PublishTrayInputStatus(m_status.asciiMode, m_status.fullShape, m_status.chinesePunctuation);
}

void CSrfTip::SyncCandidateContextState(const CandidatePageLayoutMetrics* layout) {
  if (m_candSel != m_lastSyncedCandSel || m_candPage != m_lastSyncedCandPage) {
    ++m_candidateInteractionVersion;
    m_lastSyncedCandSel = m_candSel;
    m_lastSyncedCandPage = m_candPage;
  }
  if (m_candidates.empty() || ShouldSuppressCandidatesForPrivacy()) {
    m_context.candidates.currentPage = 0;
    m_context.candidates.totalPages = 0;
    m_context.candidates.highlighted = 0;
    m_context.candidates.isLastPage = false;
    return;
  }

  CandidatePageLayoutMetrics ownedLayout;
  const CandidatePageLayoutMetrics& resolvedLayout = [&]() -> const CandidatePageLayoutMetrics& {
    if (layout) return *layout;
    ownedLayout = BuildCandidatePageLayout();
    return ownedLayout;
  }();

  const UINT totalPages = std::max(1u, static_cast<UINT>(resolvedLayout.pageStarts.size()));
  m_context.candidates.currentPage = std::min(m_candPage, totalPages - 1);
  m_context.candidates.totalPages = totalPages;
  m_context.candidates.highlighted =
      std::min(m_candSel, static_cast<UINT>(m_candidates.size() - 1));
  m_context.candidates.isLastPage = m_context.candidates.currentPage + 1 >= totalPages;
}

void CSrfTip::RebuildContextModel() {
  m_context.Clear();

  if (!m_reading.empty()) {
    m_context.preedit.str = BuildCompositionDisplay();
    SrfTextAttribute attr = {};
    attr.range.start = 0;
    attr.range.end = m_context.preedit.str.size();
    attr.range.cursor =
        static_cast<int>(std::min(m_readingCursor, m_context.preedit.str.size()));
    attr.type = SrfTextAttributeType::Highlighted;
    m_context.preedit.attributes.push_back(attr);
  }

  if (!m_candidates.empty() && !ShouldSuppressCandidatesForPrivacy()) {
    // 缓存一次布局指标，避免 CandidatePageCount/CandidateIndexInPage 在循环内反复构建布局。
    const CandidatePageLayoutMetrics layout = BuildCandidatePageLayout();
    SyncCandidateContextState(&layout);

    size_t pageIdx = 0;
    size_t nextStartIdx = layout.pageStarts.size() >= 2 ? static_cast<size_t>(layout.pageStarts[1])
                                                        : static_cast<size_t>(-1);

    for (size_t i = 0; i < m_candidates.size(); ++i) {
      if (nextStartIdx != static_cast<size_t>(-1) && i == nextStartIdx) {
        ++pageIdx;
        nextStartIdx = (pageIdx + 1 < layout.pageStarts.size())
                           ? static_cast<size_t>(layout.pageStarts[pageIdx + 1])
                           : static_cast<size_t>(-1);
      }
      const UINT indexInPage =
          static_cast<UINT>(i - static_cast<size_t>(layout.pageStarts[pageIdx]));

      CandidateMetaParts meta;
      if (i < m_candidateMeta.size() && !m_candidateMeta[i].empty()) {
        meta = SplitCandidateMeta(m_candidateMeta[i]);
      }

      SrfText item = {};
      item.str = meta.display.empty() ? m_candidates[i] : meta.display;
      PrefixCandidateDisplayText(&item.str, meta, m_uiStyle);
      m_context.candidates.items.push_back(std::move(item));

      SrfText label = {};
      label.str = std::to_wstring(indexInPage + 1);
      m_context.candidates.labels.push_back(std::move(label));

      SrfText comment = {};
      std::wstring cmt;
      if (meta.clipboardQuick) {
        auto appendClipboardDetail = [&](const std::wstring& detail) {
          if (detail.empty()) return;
          if (!cmt.empty()) cmt += L" \u00b7 ";
          cmt += detail;
        };
        if (meta.clipboardPinned) appendClipboardDetail(L"\u7f6e\u9876");
        appendClipboardDetail(meta.clipboardSource);
        appendClipboardDetail(meta.clipboardTime);
        if (!meta.clipboardType.empty()) {
          cmt.push_back(L'\t');
          cmt += meta.clipboardType;
        }
      } else if (m_uiStyle.showCandidateReading && !m_reading.empty()) {
        AppendCommentPart(&cmt, m_reading);
      }
      if (!meta.clipboardQuick && m_uiStyle.showCandidateSource) {
        AppendCommentPart(&cmt, CandidateSourceComment(meta));
      }
      if (!meta.annotation.empty()) {
        AppendCommentPart(&cmt, meta.annotation);
      }
      if (m_uiStyle.showCandidateScore && !meta.score.empty()) {
        AppendCommentPart(&cmt, meta.score);
      }
      comment.str = std::move(cmt);
      m_context.candidates.comments.push_back(std::move(comment));
    }
  }

  SyncStatusModel();
}

std::wstring CSrfTip::FocusedProcessName() {
  HWND hwnd = ResolveContextWindow(m_pFocusContext);
  if (!hwnd) {
    m_cachedFocusedHwnd = nullptr;
    m_cachedFocusedProcessId = 0;
    m_cachedFocusedProcessName.clear();
    return {};
  }

  DWORD processId = 0;
  GetWindowThreadProcessId(hwnd, &processId);
  if (processId == 0) {
    m_cachedFocusedHwnd = hwnd;
    m_cachedFocusedProcessId = 0;
    m_cachedFocusedProcessName.clear();
    return {};
  }
  if (hwnd == m_cachedFocusedHwnd && processId == m_cachedFocusedProcessId) {
    return m_cachedFocusedProcessName;
  }

  HANDLE process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, processId);
  if (!process) {
    m_cachedFocusedHwnd = hwnd;
    m_cachedFocusedProcessId = processId;
    m_cachedFocusedProcessName.clear();
    return {};
  }

  std::wstring path(MAX_PATH, L'\0');
  DWORD size = static_cast<DWORD>(path.size());
  std::wstring name;
  if (QueryFullProcessImageNameW(process, 0, path.data(), &size) && size > 0) {
    path.resize(size);
    name = path;
  }
  CloseHandle(process);
  m_cachedFocusedHwnd = hwnd;
  m_cachedFocusedProcessId = processId;
  m_cachedFocusedProcessName = name;
  return name;
}

SrfFocusSnapshot CSrfTip::CaptureFocusSnapshot(ITfContext* context) const {
  ITfContext* resolvedContext = context ? context : (m_pCompositionContext ? m_pCompositionContext
                                                                           : m_pFocusContext);
  SrfFocusSnapshot snapshot = {};
  snapshot.contextCookie = reinterpret_cast<uintptr_t>(resolvedContext);
  snapshot.hwnd = resolvedContext ? ResolveContextWindow(resolvedContext) : GetForegroundWindow();
  if (snapshot.hwnd) {
    HWND root = RootWindowForPlacement(snapshot.hwnd);
    if (root) snapshot.hwnd = root;
    (void)GetWindowThreadProcessId(snapshot.hwnd, &snapshot.processId);
  }
  snapshot.processName = m_activeAppName;
  snapshot.generation = m_focusGeneration;
  return snapshot;
}

bool CSrfTip::FocusSnapshotMatches(const SrfFocusSnapshot& snapshot) const {
  if (snapshot.generation != m_focusGeneration) return false;

  const SrfFocusSnapshot current =
      CaptureFocusSnapshot(m_pCompositionContext ? m_pCompositionContext : m_pFocusContext);
  if (snapshot.contextCookie != 0 && current.contextCookie != 0 &&
      snapshot.contextCookie != current.contextCookie) {
    return false;
  }
  if (snapshot.hwnd && current.hwnd && snapshot.hwnd != current.hwnd) return false;
  if (snapshot.processId != 0 && current.processId != 0 &&
      snapshot.processId != current.processId) {
    return false;
  }
  if (!snapshot.processName.empty() && !current.processName.empty() &&
      !WildcardMatchNoCase(snapshot.processName, current.processName)) {
    return false;
  }
  return true;
}

std::wstring CSrfTip::FormatFocusSnapshotForLog(const SrfFocusSnapshot& snapshot) const {
  std::wstring line = L"{gen=";
  line += std::to_wstring(snapshot.generation);
  line += L",ctx=";
  line += FormatPointerForLog(reinterpret_cast<const void*>(snapshot.contextCookie));
  line += L",hwnd=";
  line += FormatPointerForLog(snapshot.hwnd);
  line += L",pid=";
  line += std::to_wstring(snapshot.processId);
  line += L",app=";
  line += SanitizeDiagnosticValue(snapshot.processName);
  line += L"}";
  return line;
}

void CSrfTip::ShowNotification(SrfNotificationKind kind, const std::wstring& text) {
  RefreshCompatibilityState();
  if (ShouldHideUiForCompatibility()) {
    SrfTsfDiagnosticLog(L"notification.skip", L"compatibility policy hides UI");
    return;
  }
  const RECT* anchor = m_hasLastCandidateRect ? &m_lastCandidateRect : nullptr;
  std::wstring line = L"anchor=";
  line += anchor ? FormatRectForLog(*anchor) : L"(none)";
  line += L", text=";
  line += ShortenForLog(text, 96);
  SrfTsfDiagnosticLog(L"notification.show", line.c_str());
  SrfNotificationTone tone = SrfNotificationTone::Default;
  if (kind == SrfNotificationKind::Ime) {
    if (text.find(L"\u82f1\u6587") != std::wstring::npos) {
      tone = SrfNotificationTone::English;
    } else if (text.find(L"\u4e2d\u6587") != std::wstring::npos) {
      tone = SrfNotificationTone::Chinese;
    }
  }
  m_notificationWindow.Show(text, anchor, m_config.showNotificationsTimeMs, tone);
}

void CSrfTip::MaybeNotifyEngineHealth() {
  if (m_reading.empty()) return;
  if (!m_candidates.empty()) return;

  const SrfEngineState engineState = SrfTip_GetEngineState();
  if (engineState == SrfEngineState::Ready) {
    SrfTsfDiagnosticLog(L"engine-health.skip", L"engine is ready");
    m_engineHealthNotifiedThisComposition = false;
    return;
  }

  m_engineHealthNotifiedThisComposition = true;
  std::wstring line = L"state=";
  line += EngineStateName(engineState);
  const std::wstring failure = SrfTip_GetEngineFailureDetail();
  if (!failure.empty()) {
    line += L", failure=";
    line += ShortenForLog(failure, 96);
  }
  line += L", ui=tray";
  SrfTsfDiagnosticLog(L"engine-health.silent", line.c_str());
}

void CSrfTip::MoveReadingCaretBySyllable(bool forward) {
  std::array<uint32_t, 48> bounds{};
  const size_t n = SrfTip_SyllableBoundaryOffsetsUtf16(m_reading.c_str(), m_reading.size(), bounds.data(),
                                                       bounds.size());
  const uint32_t cur = static_cast<uint32_t>(m_readingCursor);
  if (n < 2) {
    if (forward) {
      if (m_readingCursor < m_reading.size()) ++m_readingCursor;
    } else if (m_readingCursor > 0) {
      --m_readingCursor;
    }
    return;
  }
  if (forward) {
    for (size_t i = 0; i < n; ++i) {
      if (bounds[i] > cur) {
        m_readingCursor = static_cast<size_t>(bounds[i]);
        return;
      }
    }
  } else {
    size_t i = n;
    while (i > 0) {
      --i;
      if (bounds[i] < cur) {
        m_readingCursor = static_cast<size_t>(bounds[i]);
        return;
      }
    }
  }
}

bool CSrfTip::LoadGlobalAsciiState() const {
  bool asciiMode = false;
  return TryLoadGlobalAsciiState(&asciiMode) && asciiMode;
}

bool CSrfTip::TryLoadGlobalAsciiState(bool* asciiMode) const {
  if (!asciiMode) return false;
  DWORD value = 0;
  DWORD cb = sizeof(value);
  const LONG status =
      RegGetValueW(HKEY_CURRENT_USER, kStateRegPath, kStateAsciiValue, RRF_RT_REG_DWORD, nullptr,
                   &value, &cb);
  if (status != ERROR_SUCCESS) return false;
  *asciiMode = value != 0;
  return true;
}

void CSrfTip::SaveGlobalAsciiState(bool asciiMode) const {
  HKEY key = nullptr;
  if (RegCreateKeyExW(HKEY_CURRENT_USER, kStateRegPath, 0, nullptr, 0, KEY_WRITE, nullptr, &key,
                      nullptr) != ERROR_SUCCESS) {
    return;
  }

  const DWORD value = asciiMode ? 1u : 0u;
  const bool wrote =
      RegSetValueExW(key, kStateAsciiValue, 0, REG_DWORD,
                     reinterpret_cast<const BYTE*>(&value), sizeof(value)) == ERROR_SUCCESS;
  RegCloseKey(key);
  if (wrote) NotifyTrayAsciiStateChanged(asciiMode);
}

void CSrfTip::ApplyGlobalAsciiStateFromRegistry() {
  const SrfAppOptions* options = FindAppOptions(m_config, m_activeAppName);
  if (options && options->hasAsciiMode) return;

  bool asciiMode = false;
  if (!TryLoadGlobalAsciiState(&asciiMode)) return;
  const bool nextImeOpen = !asciiMode;
  if (nextImeOpen == m_imeOpen || !m_reading.empty()) return;

  m_imeOpen = nextImeOpen;
  ApplyDefaultPunctuationForImeMode();
  SyncCompartmentState();
  SyncStatusModel();
  RebuildContextModel();
  RedrawCandidateUi();
}
