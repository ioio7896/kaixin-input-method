namespace {

class SrfScopedPerfTimer {
 public:
  explicit SrfScopedPerfTimer(const wchar_t* stage) : m_stage(stage), m_start(GetTickCount64()) {}
  ~SrfScopedPerfTimer() { DebugLogPerfMs(m_stage, m_start); }

 private:
  const wchar_t* m_stage = nullptr;
  ULONGLONG m_start = 0;
};

}  // namespace

namespace {

bool IsShiftedDirectSymbolText(const std::wstring& text) {
  if (text.size() != 1) return false;
  const wchar_t ch = text[0];
  if (ch == L' ') return false;
  if ((ch >= L'0' && ch <= L'9') || (ch >= L'a' && ch <= L'z') ||
      (ch >= L'A' && ch <= L'Z')) {
    return false;
  }
  return IsDirectInsertPreferredChar(ch);
}

std::wstring JsonEscapeTranslationText(const std::wstring& text) {
  std::wstring escaped;
  escaped.reserve(text.size() + 16);
  for (wchar_t ch : text) {
    switch (ch) {
      case L'\\': escaped += L"\\\\"; break;
      case L'\"': escaped += L"\\\""; break;
      case L'\r': escaped += L"\\r"; break;
      case L'\n': escaped += L"\\n"; break;
      case L'\t': escaped += L"\\t"; break;
      default:
        if (ch < 0x20) {
          wchar_t encoded[7] = {};
          swprintf_s(encoded, L"\\u%04x", static_cast<unsigned>(ch));
          escaped += encoded;
        } else {
          escaped.push_back(ch);
        }
    }
  }
  return escaped;
}

std::filesystem::path FindTranslationResultHelper() {
  std::filesystem::path current = ModuleDirFromAddress(
      reinterpret_cast<const void*>(&FindTranslationResultHelper));
  std::error_code ec;
  for (int depth = 0; depth < 6 && !current.empty(); ++depth) {
    const auto candidate = current / L"srf_ime_translate_result.exe";
    if (std::filesystem::is_regular_file(candidate, ec)) return candidate;
    ec.clear();
    current = current.parent_path();
  }
  return {};
}

bool LaunchCandidateTranslation(const std::wstring& text, HWND targetHwnd,
                                uint64_t focusGeneration) {
  if (text.empty() || !targetHwnd) return false;
  const auto helper = FindTranslationResultHelper();
  if (helper.empty()) return false;

  wchar_t localAppData[MAX_PATH] = {};
  const DWORD localLen = GetEnvironmentVariableW(L"LOCALAPPDATA", localAppData, MAX_PATH);
  if (localLen == 0 || localLen >= MAX_PATH) return false;
  const std::wstring requestId = std::to_wstring(GetCurrentProcessId()) + L"-" +
                                 std::to_wstring(GetTickCount64()) + L"-candidate";
  const auto requestDir = std::filesystem::path(localAppData) / L"kaixin" /
                          L"translation-requests";
  const auto requestPath = requestDir / (requestId + L".json");
  std::error_code ec;
  std::filesystem::create_directories(requestDir, ec);
  if (ec) return false;

  DWORD targetProcessId = 0;
  GetWindowThreadProcessId(targetHwnd, &targetProcessId);
  std::wstring json = L"{\"protocol_version\":2,\"request_id\":\"" + requestId +
      L"\",\"action\":\"translate\",\"text\":\"" + JsonEscapeTranslationText(text) +
      L"\",\"source\":\"auto\",\"target\":\"auto-opposite\","
      L"\"origin\":\"kaixin-ime-candidate\",\"target_hwnd\":" +
      std::to_wstring(reinterpret_cast<uintptr_t>(targetHwnd)) +
      L",\"target_process_id\":" + std::to_wstring(targetProcessId) +
      L",\"result_action\":\"paste\",\"interactive\":false,"
      L"\"presentation\":\"compact\",\"delivery\":\"return\","
      L"\"focus_generation\":" + std::to_wstring(focusGeneration) +
      L",\"replace_selection\":false}";
  AppendWideUtf8TextRaw(requestPath, json);
  if (!std::filesystem::is_regular_file(requestPath, ec)) return false;

  const std::wstring parameters = L"--request-file \"" + requestPath.wstring() + L"\"";
  const HINSTANCE launched = ShellExecuteW(nullptr, L"open", helper.c_str(), parameters.c_str(),
                                           helper.parent_path().c_str(), SW_SHOWNORMAL);
  if (reinterpret_cast<INT_PTR>(launched) <= 32) {
    std::filesystem::remove(requestPath, ec);
    return false;
  }
  return true;
}

}  // namespace

bool CSrfTip::WouldEatKey(UINT vk) {
  SrfScopedPerfTimer perf(L"Key/WouldEat");
  RefreshKeyHotPathState();
  if (IsConfiguredHotkey(vk, m_config.input.traditionalHotkey) ||
      IsConfiguredHotkey(vk, m_config.input.gameModeHotkey) ||
      IsConfiguredHotkey(vk, m_config.input.temporaryAsciiHotkey)) {
    return true;
  }
  if (ShouldForceAsciiForCompatibility()) return false;
  if (m_status.disabled) return false;

  // A game owns the physical Shift key (sprint, crouch, Steam overlay, etc.).
  // Do not consume a lone Shift for the desktop IME tap-toggle while game
  // compatibility is active; stealing only the key-down edge can also make
  // exclusive-fullscreen games lose their keyboard/focus state.
  bool appGameProfile = false;
  if (const SrfAppOptions* appOptions = FindAppOptions(m_config, CompatibilityAppName())) {
    appGameProfile = appOptions->hasGameProfile && appOptions->gameCompactProfile;
  }
  const bool gameCompatibilityActive =
      appGameProfile || m_gameCompatActive || m_configuredGameCompatActive ||
      m_builtinGameCompatActive || m_manualGameCompatActive;
  if (gameCompatibilityActive && IsVkShift(vk)) {
    m_shiftTapActive = false;
    m_shiftTapUsedWithOtherKey = false;
    return false;
  }

  // Shift-tap toggles Chinese/English.
  // 为实现「轻按 Shift 双向切中/英（无论当前模式、无论是否在输入拼音）」：
  // - Shift 本身始终拦截，以便在 KeyUp 判定轻按并执行切换。
  // - Shift 已按下后，只拦截 IME 确实要处理的后续按键。
  //   Ctrl/Alt 组合键直接让给宿主应用，避免占用编辑器和设计软件快捷键。
  if (m_config.input.shiftTapHotkeyEnabled) {
    if (IsVkShift(vk)) {
      return true;
    }
    if (m_shiftTapActive) {
      m_shiftTapUsedWithOtherKey = true;
      if (HasCtrlOrAltDown()) return false;
      if (m_reading.empty()) return m_imeOpen && ShouldHandleDirectKey(vk);
      return true;
    }
  }

  if (!m_reading.empty()) {
    const bool translateCandidate = vk == VK_RETURN &&
                                    (GetKeyState(VK_CONTROL) & 0x8000) != 0 &&
                                    (GetKeyState(VK_SHIFT) & 0x8000) != 0 &&
                                    (GetKeyState(VK_MENU) & 0x8000) == 0;
    if (translateCandidate) return true;
    const bool ctrlArrow = (GetKeyState(VK_CONTROL) & 0x8000) != 0 &&
                           (GetKeyState(VK_MENU) & 0x8000) == 0 &&
                           (vk == VK_LEFT || vk == VK_RIGHT);
    if (!ctrlArrow && HasCtrlOrAltDown()) return false;

    if (IsPrintableDirectVk(vk)) return true;
    switch (vk) {
      case VK_BACK:
      case VK_ESCAPE:
      case VK_RETURN:
      case VK_UP:
      case VK_DOWN:
      case VK_LEFT:
      case VK_RIGHT:
      case VK_HOME:
      case VK_END:
      case VK_PRIOR:
      case VK_NEXT:
      case VK_TAB:
      case VK_OEM_MINUS:
      case VK_OEM_PLUS:
        return true;
      default:
        break;
    }
    switch (vk) {
      case VK_OEM_COMMA:
      case VK_OEM_PERIOD:
      case VK_OEM_4:
      case VK_OEM_6:
        return true;
      default:
        break;
    }
    return false;
  }

  if (vk == VK_BACK) return false;
  if (!m_imeOpen) return false;
  // 右侧小键盘数字/符号在中文模式下也应像普通直输键一样交给宿主应用。
  // 若由 TSF 接管再插入，部分宿主会短暂渲染成带下划线的预编辑文本。
  if (IsNumpadPrintableVk(vk)) return false;
  return ShouldHandleDirectKey(vk);
}

HRESULT CSrfTip::ProcessKey(TfEditCookie ec, ITfContext* pic, UINT vk, LPARAM lParam,
                            bool shiftDown, bool* pHandled) {
  SrfScopedPerfTimer perf(L"Key/ProcessKey");
  if (!pHandled) return E_POINTER;
  *pHandled = false;
  if (!pic) return E_INVALIDARG;

  RefreshKeyHotPathState();
  const bool keyDownTransition = (lParam & 0x40000000) == 0;
  if (IsConfiguredHotkey(vk, m_config.input.traditionalHotkey)) {
    if (keyDownTransition) ToggleTraditionalOutput();
    *pHandled = true;
    return S_OK;
  }
  if (IsConfiguredHotkey(vk, m_config.input.gameModeHotkey)) {
    if (keyDownTransition) ToggleManualGameCompat();
    *pHandled = true;
    return S_OK;
  }
  if (IsConfiguredHotkey(vk, m_config.input.temporaryAsciiHotkey)) {
    if (keyDownTransition) ToggleManualAsciiMode(ec);
    *pHandled = true;
    return S_OK;
  }
  if (ShouldForceAsciiForCompatibility()) {
    m_compatibilityAsciiCleanupPending = false;
    if (m_candidateUi) m_candidateUi->End();
    if (m_pComposition) {
      CancelCompositionEdit(ec);
    } else {
      ReleaseCompositionState();
    }
    RebuildContextModel();
    SyncStatusModel();
    *pHandled = false;
    return S_OK;
  }
  ClampReadingCursor();

  const bool translateCandidate = !m_reading.empty() && vk == VK_RETURN &&
                                  (GetKeyState(VK_CONTROL) & 0x8000) != 0 &&
                                  (GetKeyState(VK_SHIFT) & 0x8000) != 0 &&
                                  (GetKeyState(VK_MENU) & 0x8000) == 0;
  if (translateCandidate) {
    if (keyDownTransition) {
      const std::wstring candidate = m_candSel < m_candidates.size()
                                         ? m_candidates[m_candSel]
                                         : m_reading;
      const HWND target = GetForegroundWindow();
      if (LaunchCandidateTranslation(candidate, target, m_compositionGeneration)) {
        CancelCompositionEdit(ec);
        RebuildContextModel();
        SyncStatusModel();
      } else {
        MessageBeep(MB_ICONERROR);
      }
    }
    *pHandled = true;
    return S_OK;
  }

  // Shift-tap toggles Chinese/English。
  // lParam bit31：0=按下 1=释放（KeyUp 上完成轻按判定，见 key_sink OnKeyUp）。
  if (m_config.input.shiftTapHotkeyEnabled && IsVkShift(vk)) {
    const bool wasDown = (lParam & 0x80000000) != 0;
    if (!wasDown) {
      m_shiftTapActive = true;
      m_shiftTapUsedWithOtherKey = false;
      m_shiftTapStartTick = GetTickCount64();
      *pHandled = true;
      return S_OK;
    }

    const ULONGLONG now = GetTickCount64();
    const ULONGLONG heldMs = now - m_shiftTapStartTick;
    const bool shouldToggle =
        m_shiftTapActive && !m_shiftTapUsedWithOtherKey && heldMs >= 30 && heldMs <= 420 && !HasCtrlOrAltDown();
    m_shiftTapActive = false;
    m_shiftTapUsedWithOtherKey = false;

    if (shouldToggle) {
      if (m_imeOpen && !m_reading.empty()) {
        const HRESULT hr = CommitReadingText(ec, pic);
        *pHandled = true;
        if (FAILED(hr)) return hr;
      }
      ToggleImeOpen();
    }
    *pHandled = true;
    return S_OK;
  } else if (m_config.input.shiftTapHotkeyEnabled && m_shiftTapActive) {
    // WouldEatKey 已经避开 Ctrl/Alt 组合；进入这里的后续键均是 IME 需要处理的键。
    m_shiftTapUsedWithOtherKey = true;
    if (!m_imeOpen && m_reading.empty()) {
      *pHandled = false;
      return S_OK;
    }
  }

  if (!m_reading.empty()) {
    auto invalidateUserPhraseCompose = [&]() {
      const bool hadUserPhraseCompose = m_userPhraseComposeActive || m_userPhraseComposeValid ||
                                        !m_userPhraseComposeCommitted.empty();
      m_userPhraseComposeActive = false;
      m_userPhraseComposeValid = false;
      m_userPhraseComposeOriginalReading.clear();
      m_userPhraseComposeCommitted.clear();
      ApplyRustModeFlags();
      if (hadUserPhraseCompose) SrfTip_ResetLearningContext();
    };

    if (shiftDown && !HasCtrlOrAltDown() &&
        (IsLetterVk(vk) ||
         (m_config.input.shiftSymbolTemporaryAscii && (IsDigitVk(vk) || IsOemPrintableVk(vk))))) {
      std::wstring directAscii = TranslateDirectKey(vk, lParam, shiftDown);
      if (directAscii.empty() && IsLetterVk(vk)) {
        wchar_t ch = 0;
        if (vk >= 'a' && vk <= 'z') {
          ch = static_cast<wchar_t>(vk - 'a' + L'A');
        } else if (vk >= 'A' && vk <= 'Z') {
          ch = static_cast<wchar_t>(vk);
        }
        if (ch != 0) directAscii.assign(1, ch);
      }
      if (!directAscii.empty()) {
        invalidateUserPhraseCompose();
        const HRESULT hrCommit = CommitReadingText(ec, pic);
        if (FAILED(hrCommit)) {
          *pHandled = true;
          return hrCommit;
        }
        // CommitCandidate may have started a continuation composition (single-char
        // commit with remaining syllables). Cancel and release it for clean insertion.
        if (m_pComposition) {
          CancelCompositionEdit(ec);
          ReleaseCompositionObjects();
        }
        // Shift with letters, and optionally symbols, is temporary ASCII input.
        const HRESULT hr = CommitDirectText(ec, pic, directAscii);
        *pHandled = true;
        return hr;
      }
    }

    std::wstring directText;
    bool directTextResolved = false;
    auto resolveDirectText = [&]() -> const std::wstring& {
      if (!directTextResolved) {
        directText = TranslateDirectKey(vk, lParam, shiftDown);
        directTextResolved = true;
      }
      return directText;
    };
    auto commitShiftedDirectSymbol = [&]() -> HRESULT {
      const std::wstring rawText = resolveDirectText();
      if (!shiftDown || HasCtrlOrAltDown() || !IsShiftedDirectSymbolText(rawText)) {
        return S_FALSE;
      }
      invalidateUserPhraseCompose();
      *pHandled = true;
      // Shift-number/OEM is handled before candidate number selection. By default
      // it still follows Chinese punctuation/fullwidth conversion; users can opt
      // into old temporary-ASCII behavior with shift_symbol_temporary_ascii=1.
      if (m_config.input.shiftSymbolTemporaryAscii) {
        return CommitReadingThenDirectTextWithCursor(ec, pic, rawText, -1);
      }
      LONG cursorOffset = -1;
      std::wstring output = ConvertDirectTextWithCompletion(rawText, &cursorOffset);
      return CommitReadingThenDirectTextWithCursor(ec, pic, output, cursorOffset);
    };
    auto currentCandidatesContainPrefixPlaceholder = [&]() {
      for (const auto& rawMeta : m_candidateMeta) {
        if (!rawMeta.empty() && SplitCandidateMeta(rawMeta).prefixPlaceholder) return true;
      }
      return false;
    };
    auto candidatesReadyForCurrentReading = [&]() {
      return !m_candidates.empty() && m_candidatesReading == m_reading &&
             !currentCandidatesContainPrefixPlaceholder();
    };
    auto selectionCandidatesReadyForCurrentReading = [&]() {
      if (candidatesReadyForCurrentReading()) return true;
      if (EnsureBlindCommitCandidatesReady()) return true;
      RefreshCandidates();
      return candidatesReadyForCurrentReading();
    };

    const HRESULT shiftedSymbolHr = commitShiftedDirectSymbol();
    if (shiftedSymbolHr != S_FALSE) return shiftedSymbolHr;

    const wchar_t digitCh = DigitCharFromVk(vk);
    if (digitCh != 0 && IsFunctionKeyTokenPrefix(m_reading)) {
      std::wstring next = m_reading;
      next.insert(m_readingCursor, 1, digitCh);
      if (IsValidFunctionKeyToken(next)) {
        invalidateUserPhraseCompose();
        m_reading = std::move(next);
        ++m_readingCursor;
        *pHandled = true;
        return SyncCompositionText(ec, pic, true);
      }
    }

    HRESULT clipboardQuickPageHr = S_OK;
    auto applyClipboardQuickPageControl = [&](bool nextPage) -> bool {
      UINT page = 0;
      std::wstring filter;
      if (!ParseClipboardQuickReading(m_reading, &page, &filter)) return false;
      const bool hasQuickCandidates = std::any_of(
          m_candidateMeta.begin(), m_candidateMeta.end(),
          [](const std::wstring& meta) { return IsClipboardQuickMeta(meta); });
      if (!hasQuickCandidates && page == 0 && m_reading != L"vvu") return false;

      if (!nextPage && page == 0) return true;
      if (nextPage) {
        for (const auto& rawMeta : m_candidateMeta) {
          if (rawMeta.empty()) continue;
          const CandidateMetaParts meta = SplitCandidateMeta(rawMeta);
          if (meta.clipboardQuick && page + 1 >= meta.clipboardPages) return true;
        }
      }
      const UINT targetPage = nextPage ? page + 1 : page - 1;
      m_reading = BuildClipboardQuickReading(targetPage, filter);
      m_readingCursor = m_reading.size();
      clipboardQuickPageHr = SyncCompositionText(ec, pic, true);
      return true;
    };

    switch (vk) {
      case VK_BACK:
        invalidateUserPhraseCompose();
        if (m_readingCursor > 0) {
          m_reading.erase(m_readingCursor - 1, 1);
          --m_readingCursor;
          if (m_reading.empty()) {
            CancelCompositionEdit(ec);
          } else {
            (void)SyncCompositionText(ec, pic, true);
          }
          *pHandled = true;
        } else {
          *pHandled = true;
        }
        return S_OK;
      case VK_ESCAPE:
        invalidateUserPhraseCompose();
        CancelCompositionEdit(ec);
        *pHandled = true;
        return S_OK;
      case VK_LEFT: {
        invalidateUserPhraseCompose();
        const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
        if (ctrl) {
          MoveReadingCaretBySyllable(false);
        } else if (m_readingCursor > 0) {
          --m_readingCursor;
        }
        RebuildContextModel();
        UpdateCandidateWindow(ec);
        *pHandled = true;
        return S_OK;
      }
      case VK_RIGHT: {
        invalidateUserPhraseCompose();
        const bool ctrl = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
        if (ctrl) {
          MoveReadingCaretBySyllable(true);
        } else if (m_readingCursor < m_reading.size()) {
          ++m_readingCursor;
        }
        RebuildContextModel();
        UpdateCandidateWindow(ec);
        *pHandled = true;
        return S_OK;
      }
      case VK_HOME:
        invalidateUserPhraseCompose();
        m_readingCursor = 0;
        RebuildContextModel();
        UpdateCandidateWindow(ec);
        *pHandled = true;
        return S_OK;
      case VK_END:
        invalidateUserPhraseCompose();
        m_readingCursor = m_reading.size();
        RebuildContextModel();
        UpdateCandidateWindow(ec);
        *pHandled = true;
        return S_OK;
      case VK_UP:
        if (!candidatesReadyForCurrentReading()) {
          *pHandled = true;
          return S_OK;
        }
        if (m_candSel > 0) --m_candSel;
        ClampCandidateState();
        SyncCandidateContextState();
        RedrawCandidateUi();
        *pHandled = true;
        return S_OK;
      case VK_DOWN:
        if (!candidatesReadyForCurrentReading()) {
          *pHandled = true;
          return S_OK;
        }
        if (m_candSel + 1 < m_candidates.size()) ++m_candSel;
        ClampCandidateState();
        SyncCandidateContextState();
        RedrawCandidateUi();
        *pHandled = true;
        return S_OK;
      case VK_PRIOR:
        if (!candidatesReadyForCurrentReading()) {
          *pHandled = true;
          return S_OK;
        }
        if (m_candPage == 0 && applyClipboardQuickPageControl(false)) {
          *pHandled = true;
          return clipboardQuickPageHr;
        }
        if (!m_config.input.pagePgUpDown) {
          *pHandled = true;
          return S_OK;
        }
        if (m_candPage > 0) {
          const UINT offset = CandidateIndexInPage(m_candSel);
          --m_candPage;
          const UINT pageStart = CandidatePageStart(m_candPage);
          const UINT pageEndExclusive = CandidatePageEndExclusive(m_candPage);
          m_candSel = std::min(pageStart + offset, pageEndExclusive - 1);
          ClampCandidateState();
          SyncCandidateContextState();
          RedrawCandidateUi();
        }
        *pHandled = true;
        return S_OK;
      case VK_NEXT:
        if (!candidatesReadyForCurrentReading()) {
          *pHandled = true;
          return S_OK;
        }
        if (m_candPage >= MaxCandidatePage() && applyClipboardQuickPageControl(true)) {
          *pHandled = true;
          return clipboardQuickPageHr;
        }
        if (!m_config.input.pagePgUpDown) {
          *pHandled = true;
          return S_OK;
        }
        if (m_candPage < MaxCandidatePage()) {
          const UINT offset = CandidateIndexInPage(m_candSel);
          ++m_candPage;
          const UINT pageStart = CandidatePageStart(m_candPage);
          const UINT pageEndExclusive = CandidatePageEndExclusive(m_candPage);
          m_candSel = std::min(pageStart + offset, pageEndExclusive - 1);
          ClampCandidateState();
          SyncCandidateContextState();
          RedrawCandidateUi();
        }
        *pHandled = true;
        return S_OK;
      case VK_OEM_MINUS:
        if (!m_candidates.empty()) {
          if (applyClipboardQuickPageControl(false)) {
            *pHandled = true;
            return clipboardQuickPageHr;
          }
          if (m_config.input.pageMinusEqual) {
            if (m_candPage > 0) {
              const UINT offset = CandidateIndexInPage(m_candSel);
              --m_candPage;
              const UINT pageStart = CandidatePageStart(m_candPage);
              const UINT pageEndExclusive = CandidatePageEndExclusive(m_candPage);
              m_candSel = std::min(pageStart + offset, pageEndExclusive - 1);
              ClampCandidateState();
              SyncCandidateContextState();
              RedrawCandidateUi();
            }
            *pHandled = true;
            return S_OK;
          }
          break;
        }
        break;
      case VK_OEM_PLUS:
        if (!m_candidates.empty()) {
          if (applyClipboardQuickPageControl(true)) {
            *pHandled = true;
            return clipboardQuickPageHr;
          }
          if (m_config.input.pageMinusEqual) {
            if (m_candPage < MaxCandidatePage()) {
              const UINT offset = CandidateIndexInPage(m_candSel);
              ++m_candPage;
              const UINT pageStart = CandidatePageStart(m_candPage);
              const UINT pageEndExclusive = CandidatePageEndExclusive(m_candPage);
              m_candSel = std::min(pageStart + offset, pageEndExclusive - 1);
              ClampCandidateState();
              SyncCandidateContextState();
              RedrawCandidateUi();
            }
            *pHandled = true;
            return S_OK;
          }
          break;
        }
        break;
      case VK_OEM_COMMA:
      case VK_OEM_4:
        if (!m_candidates.empty()) {
          if (m_config.input.pageCommaPeriod && m_candPage > 0 && resolveDirectText().empty()) {
            const UINT offset = CandidateIndexInPage(m_candSel);
            --m_candPage;
            const UINT pageStart = CandidatePageStart(m_candPage);
            const UINT pageEndExclusive = CandidatePageEndExclusive(m_candPage);
            m_candSel = std::min(pageStart + offset, pageEndExclusive - 1);
            ClampCandidateState();
            SyncCandidateContextState();
            RedrawCandidateUi();
            *pHandled = true;
            return S_OK;
          }
          break;
        }
        break;
      case VK_OEM_PERIOD:
      case VK_OEM_6:
        if (!m_candidates.empty()) {
          if (m_config.input.pageCommaPeriod && m_candPage < MaxCandidatePage() &&
              resolveDirectText().empty()) {
            const UINT offset = CandidateIndexInPage(m_candSel);
            ++m_candPage;
            const UINT pageStart = CandidatePageStart(m_candPage);
            const UINT pageEndExclusive = CandidatePageEndExclusive(m_candPage);
            m_candSel = std::min(pageStart + offset, pageEndExclusive - 1);
            ClampCandidateState();
            SyncCandidateContextState();
            RedrawCandidateUi();
            *pHandled = true;
            return S_OK;
          }
          break;
        }
        break;
      case VK_TAB:
        if (!candidatesReadyForCurrentReading()) {
          *pHandled = true;
          return S_OK;
        }
        if (shiftDown) {
          if (m_candSel > 0) --m_candSel;
        } else if (m_candSel + 1 < m_candidates.size()) {
          ++m_candSel;
        }
        ClampCandidateState();
        SyncCandidateContextState();
        RedrawCandidateUi();
        *pHandled = true;
        return S_OK;
      case VK_SPACE: {
        *pHandled = true;
        const bool hadVisibleCandidates = !m_candidates.empty();
        const SrfEngineState stateBefore = SrfTip_GetEngineState();
        if (selectionCandidatesReadyForCurrentReading()) {
          return CommitCandidate(ec, m_candSel);
        }
        const bool shouldWaitForCandidates =
            hadVisibleCandidates || ShouldUseBlindCommitForCompatibility() ||
            stateBefore != SrfEngineState::Ready ||
            SrfTip_GetEngineState() == SrfEngineState::Loading;
        if (shouldWaitForCandidates) return S_OK;
        return CommitCandidate(ec, static_cast<size_t>(-1));
      }
      case VK_RETURN:
        *pHandled = true;
        return CommitReadingText(ec, pic);
      default:
        break;
    }

    const int candidateNumberIndex = CandidateNumberIndexFromVk(vk);
    const bool candidateNumberSelectKey =
        m_config.input.candidateNumberSelect && candidateNumberIndex >= 0 && IsDigitVk(vk);
    if (candidateNumberSelectKey) {
      *pHandled = true;
      if (!selectionCandidatesReadyForCurrentReading()) {
        std::wstring line = L"reading=";
        line += ShortenForLog(m_reading, 24);
        line += L", vk=";
        line += std::to_wstring(vk);
        line += L", reason=candidates_not_ready";
        SrfTsfDiagnosticLog(L"candidate-number-select.defer", line.c_str());
        return S_OK;
      }
      const UINT pageStart = CandidatePageStart(m_candPage);
      const UINT pageEndExclusive = CandidatePageEndExclusive(m_candPage);
      const size_t idx = pageStart + static_cast<size_t>(candidateNumberIndex);
      if (idx >= pageEndExclusive) return S_OK;
      return CommitCandidate(ec, idx);
    }

    std::wstring reading = resolveDirectText();
    if (reading.empty() && IsLetterVk(vk)) reading.push_back(static_cast<wchar_t>(vk));

    if (!reading.empty()) {
      const bool continueComposition = IsLetterVk(vk) || vk == VK_OEM_7;
      if (!continueComposition) {
        invalidateUserPhraseCompose();
        *pHandled = true;
        LONG cursorOffset = -1;
        std::wstring output = ConvertDirectTextWithCompletion(std::move(reading), &cursorOffset);
        return CommitReadingThenDirectTextWithCursor(ec, pic, output, cursorOffset);
      }
      for (wchar_t& ch : reading) ch = static_cast<wchar_t>(towlower(ch));
      invalidateUserPhraseCompose();
      m_reading.insert(m_readingCursor, reading);
      m_readingCursor += reading.size();
      *pHandled = true;
      return SyncCompositionText(ec, pic, true);
    }

    return S_OK;
  }

  if (!m_imeOpen || !ShouldHandleDirectKey(vk)) return S_OK;

  if (ShouldUseTemporaryEnglish(vk, shiftDown)) {
    std::wstring text = TranslateDirectKey(vk, lParam, shiftDown);
    if (text.empty()) return S_OK;
    *pHandled = true;
    return CommitDirectText(ec, pic, text);
  }

  if (IsLetterVk(vk)) {
    std::wstring reading = TranslateDirectKey(vk, lParam, shiftDown);
    if (reading.empty()) reading.push_back(static_cast<wchar_t>(vk));
    for (wchar_t& ch : reading) ch = static_cast<wchar_t>(towlower(ch));
    m_reading = reading;
    m_readingCursor = m_reading.size();
    *pHandled = true;
    return SyncCompositionText(ec, pic, true);
  }

  std::wstring text = TranslateDirectKey(vk, lParam, shiftDown);
  LONG cursorOffset = -1;
  text = ConvertDirectTextWithCompletion(std::move(text), &cursorOffset);
  if (text.empty()) return S_OK;

  *pHandled = true;
  return CommitDirectTextWithCursor(ec, pic, text, cursorOffset);
}
