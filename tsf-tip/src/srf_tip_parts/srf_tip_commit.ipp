CSrfTip::SrfPhraseComposeStateGuard::SrfPhraseComposeStateGuard(CSrfTip* tip)
    : tip_(tip),
      wasActive_(tip->m_userPhraseComposeActive),
      wasValid_(tip->m_userPhraseComposeValid),
      originalReading_(tip->m_userPhraseComposeOriginalReading),
      committed_(tip->m_userPhraseComposeCommitted) {}

CSrfTip::SrfPhraseComposeStateGuard::~SrfPhraseComposeStateGuard() {
  tip_->m_userPhraseComposeActive = wasActive_;
  tip_->m_userPhraseComposeValid = wasValid_;
  tip_->m_userPhraseComposeOriginalReading = originalReading_;
  tip_->m_userPhraseComposeCommitted = committed_;
}

HRESULT CSrfTip::CommitCandidate(TfEditCookie ec, size_t idx, bool explicitSelection) {
  return CommitCandidateResolved(ec, nullptr, idx, nullptr, nullptr, nullptr, nullptr,
                                 explicitSelection);
}

HRESULT CSrfTip::CommitCandidateSnapshot(TfEditCookie ec, ITfContext* requestContext, size_t idx,
                                         const std::wstring& reading,
                                         const std::wstring& committedText,
                                         const std::wstring& metaText,
                                         const std::vector<std::wstring>& skippedCandidates,
                                         bool explicitSelection) {
  if (!reading.empty() && (m_reading != reading || m_candidatesReading != reading)) {
    std::wstring line = L"idx=" + std::to_wstring(idx);
    line += L", snapshot=";
    line += ShortenForLog(reading, 24);
    line += L", current=";
    line += ShortenForLog(m_reading, 24);
    line += L", candidatesReading=";
    line += ShortenForLog(m_candidatesReading, 24);
    SrfTsfDiagnosticLog(L"commit-candidate.drop-stale", line.c_str());
    return S_OK;
  }
  return CommitCandidateResolved(ec, requestContext, idx, &reading, &committedText, &metaText,
                                 &skippedCandidates, explicitSelection);
}

HRESULT CSrfTip::CommitCandidateResolved(TfEditCookie ec, ITfContext* requestContext, size_t idx,
                                         const std::wstring* snapshotReading,
                                         const std::wstring* snapshotCommitted,
                                         const std::wstring* snapshotMeta,
                                         const std::vector<std::wstring>* snapshotSkippedCandidates,
                                         bool explicitSelection) {
  const ULONGLONG commitStart = GetTickCount64();
  const bool commitReading = idx == static_cast<size_t>(-1);
  auto snapshotOrCurrentReading = [&]() -> std::wstring {
    if (snapshotReading && !snapshotReading->empty()) return *snapshotReading;
    return !m_reading.empty() ? m_reading : std::wstring();
  };
  std::wstring committed =
      commitReading
          ? snapshotOrCurrentReading()
          : (snapshotCommitted && !snapshotCommitted->empty()
                 ? *snapshotCommitted
                 : (idx < m_candidates.size()
                        ? m_candidates[idx]
                        : (!m_candidates.empty() ? m_candidates.front()
                                                 : snapshotOrCurrentReading())));
  if (committed.empty()) return S_OK;

  CandidateMetaParts committedMeta;
  if (!commitReading && snapshotMeta && !snapshotMeta->empty()) {
    committedMeta = SplitCandidateMeta(*snapshotMeta);
  } else if (!commitReading && idx < m_candidateMeta.size() && !m_candidateMeta[idx].empty()) {
    committedMeta = SplitCandidateMeta(m_candidateMeta[idx]);
  }
  if (!commitReading && committedMeta.prefixPlaceholder) {
    std::wstring line = L"idx=" + std::to_wstring(idx);
    line += L", reading=";
    line += ShortenForLog(snapshotReading && !snapshotReading->empty() ? *snapshotReading : m_reading,
                          24);
    SrfTsfDiagnosticLog(L"commit-candidate.skip-placeholder", line.c_str());
    return S_OK;
  }
  const bool suppressCandidateLearning =
      committedMeta.noLearn || committedMeta.clipboardQuick || committedMeta.partialResult ||
      committedMeta.prefixPlaceholder || m_traditionalOutput || m_sensitiveInputActive ||
      ShouldForceAsciiForCompatibility() || ShouldSuppressLearningForPrivacy();
  // A partial first page is unsafe for ordinary candidate-learning feedback,
  // but an explicitly selected single character is still a valid step in the
  // user's deliberate phrase composition.  Do not let the partial marker
  // discard the whole composed phrase in that path.
  const bool suppressPhraseComposeLearning =
      committedMeta.noLearn || committedMeta.clipboardQuick || committedMeta.prefixPlaceholder ||
      m_traditionalOutput || m_sensitiveInputActive || ShouldForceAsciiForCompatibility() ||
      ShouldSuppressLearningForPrivacy();
  if (committedMeta.clipboardQuick) {
    if (m_candidateUi) m_candidateUi->End();
    if (ShouldSuppressClipboardForPrivacy()) {
      SrfTsfDiagnosticLog(L"clipboard-quick.resolve", L"reason=privacy_disabled");
      SrfTip_ResetLearningContext();
      if (m_pComposition) {
        CancelCompositionEdit(ec);
      } else {
        ReleaseCompositionState();
      }
      RebuildContextModel();
      SyncStatusModel();
      return S_OK;
    }
    const ULONGLONG clipboardStart = GetTickCount64();
    // Short entries (<= CLIPBOARD_QUICK_INLINE_TEXT_UTF16_LIMIT units) already
    // carry their full text inline in the phrase; only longer entries hold a
    // clipboard://{id} placeholder that needs the pipe round trip.  Skipping
    // the resolve for inline texts keeps the common commit offline-safe and
    // removes a blocking pipe wait from the key thread.
    if (!committedMeta.clipboardId.empty() &&
        committed.rfind(L"clipboard://", 0) == 0) {
      std::wstring resolved;
      if (!SrfTip_ResolveClipboardTextCached(committedMeta.clipboardId, &resolved)) {
        if (committed.rfind(L"clipboard://", 0) == 0) {
          SrfTsfDiagnosticLog(L"clipboard-quick.resolve", L"reason=resolve_failed");
          SrfTip_ResetLearningContext();
          if (m_pComposition) {
            CancelCompositionEdit(ec);
          } else {
            ReleaseCompositionState();
          }
          RebuildContextModel();
          SyncStatusModel();
          return S_OK;
        }
        SrfTsfDiagnosticLog(L"clipboard-quick.resolve", L"reason=resolve_failed_fallback_inline");
      } else {
        committed = std::move(resolved);
      }
    }
    DebugLogPerfMs(L"CommitCandidate/clipboard-resolve", clipboardStart);
    // Treat selecting a vvu clipboard candidate as a fresh use. The store
    // keeps history in most-recent-first order, while pinned entries retain
    // their separate priority.
    SrfTip_RecordClipboardText(committed);
  }
  const std::wstring reading =
      snapshotReading && !snapshotReading->empty() ? *snapshotReading : m_reading;
  const bool usedComposition = m_pComposition != nullptr;
  ITfContext* pic = m_pCompositionContext ? m_pCompositionContext
                                          : (requestContext ? requestContext : m_pFocusContext);
  UINT readingFunctionVk = 0;
  UINT committedFunctionVk = 0;
  const bool commitAsFunctionKey =
      TryParseFunctionKeyToken(reading, &readingFunctionVk) &&
      TryParseFunctionKeyToken(committed, &committedFunctionVk) && readingFunctionVk == committedFunctionVk;

  // 单字候选 + 至少两个完整音节：上屏一字并保留后续拼音（与引擎「前 7 多字 + 首音节单字」策略配套）。
  // ClipboardPaste has a full clipboard/OLE round trip per commit.  Do not
  // enter the single-character continuation path in game mode: repeatedly
  // selecting a long phrase would otherwise perform one expensive paste for
  // every character and make the game input feel frozen.  The normal commit
  // path still submits the selected character through the configured transport.
  bool appGameProfile = false;
  if (const SrfAppOptions* appOptions = FindAppOptions(m_config, CompatibilityAppName())) {
    appGameProfile = appOptions->hasGameProfile && appOptions->gameCompactProfile;
  }
  const bool gameCompatibilityActive =
      appGameProfile || m_gameCompatActive || m_configuredGameCompatActive ||
      m_builtinGameCompatActive || m_fullscreenCompatActive || m_manualGameCompatActive;
  const bool clipboardGameContinuation =
      EffectiveCommitTransport() == SrfCommitTransport::ClipboardPaste && gameCompatibilityActive;
  if (committed.size() == 1 && pic && !clipboardGameContinuation) {
    auto continueSingleCharCommit = [&](std::wstring restReading,
                                        const std::wstring* learnReading) -> HRESULT {
      if (m_candidateUi) m_candidateUi->End();

      ITfContext* ctx = m_pCompositionContext ? m_pCompositionContext : (pic ? pic : m_pFocusContext);
      if (!ctx) return E_FAIL;
      ctx->AddRef();

      {
        const SrfPhraseComposeStateGuard phraseComposeGuard(this);

        // 关键：先结束当前 composition。否则后续 SyncCompositionText 可能复用同一范围，
        // 把刚刚直插上屏的前一个字覆盖掉，表现为“只剩最后一次选字”。
        if (m_pComposition) {
          CancelCompositionEdit(ec);
          ReleaseCompositionObjects();
        }
      }

      auto commitSingleCharByInsert = [&]() -> HRESULT {
        ITfInsertAtSelection* insertAtSelection = nullptr;
        HRESULT hr = ctx->QueryInterface(IID_ITfInsertAtSelection,
                                         reinterpret_cast<void**>(&insertAtSelection));
        if (FAILED(hr) || !insertAtSelection) return FAILED(hr) ? hr : E_FAIL;

        ITfRange* insertedRange = nullptr;
        hr = insertAtSelection->InsertTextAtSelection(ec, 0, committed.c_str(),
                                                      static_cast<LONG>(committed.size()),
                                                      &insertedRange);
        insertAtSelection->Release();
        if (SUCCEEDED(hr) && insertedRange) {
          (void)insertedRange->Collapse(ec, TF_ANCHOR_END);
          TF_SELECTION selection = {};
          selection.range = insertedRange;
          selection.style.ase = TF_AE_NONE;
          selection.style.fInterimChar = FALSE;
          (void)ctx->SetSelection(ec, 1, &selection);
          insertedRange->Release();
        }
        if (SrfTsfDebugTraceEnabled()) {
          wchar_t buf[152] = {};
          swprintf_s(buf, L"continueSingleCharCommit direct-insert hr=0x%08lX ch=%lc",
                     static_cast<unsigned long>(hr), committed[0]);
          SrfTsfDebugLog(buf);
        }
        return hr;
      };

      // 逐字上屏必须是“稳定的直接插入”，避免随后 SyncCompositionText 复用 composition range 覆盖已选字导致吞字。
      HRESULT hrIns = EffectiveCommitTransport() != SrfCommitTransport::Tsf
                          ? CommitDirectText(ec, ctx, committed)
                          : commitSingleCharByInsert();
      if (FAILED(hrIns) && EffectiveCommitTransport() == SrfCommitTransport::Tsf) {
        // 个别宿主可能不支持 InsertTextAtSelection，兜底沿用旧路径。
        hrIns = CommitDirectText(ec, ctx, committed);
      }
      if (FAILED(hrIns)) {
        ctx->Release();
        return hrIns;
      }

      if (!suppressPhraseComposeLearning) {
        // 启动/继续“用户造词”累计：用最初 reading 作为 key，把逐字上屏的结果拼成一个整词写入用户词典。
        if (!m_userPhraseComposeActive) {
          std::wstring trimmedOrig = reading;
          TrimWstringInPlace(trimmedOrig);
          m_userPhraseComposeOriginalReading = std::move(trimmedOrig);
          m_userPhraseComposeCommitted.clear();
          m_userPhraseComposeActive = !m_userPhraseComposeOriginalReading.empty();
          m_userPhraseComposeValid = m_userPhraseComposeActive;
        }
        if (m_userPhraseComposeActive) {
          m_userPhraseComposeCommitted += committed;
          if (m_userPhraseComposeCommitted.size() > 8) m_userPhraseComposeValid = false;
        }

        if (learnReading && !learnReading->empty()) {
          SrfTip_LearnCommitEx(*learnReading, committed, kSrfLearnCommitWeak);
        }
      }
      ApplyRustModeFlags();

      m_reading = std::move(restReading);
      // 逐字上屏后，继续输入应接在剩余拼音末尾，行为与主流拼音输入法一致。
      m_readingCursor = m_reading.size();
      // 保留旧候选作为不可点击的短暂占位（m_candidatesReading 与 reading
      // 不匹配时 UI 会拒绝提交）。这样不会在首字上屏后闪灭/跳位。
      if (m_hasLastCandidateRect) m_preserveCandidateAnchorReading = m_reading;
      SetCandidateViewState(m_candidates.empty() ? SrfCandidateViewState::Pending
                                                  : SrfCandidateViewState::Stale,
                            L"commit-rest-reading");
      m_candSel = 0;
      m_candPage = 0;

      const HRESULT hrSync = SyncCompositionText(ec, ctx, false);
      ctx->Release();
      if (FAILED(hrSync)) {
        // 已经成功上屏 committed，剩余 preedit 同步失败时，清理本地状态避免残留“幽灵拼音”。
        ReleaseCompositionState();
        return hrSync;
      }
      ScheduleDeferredCandidateRefresh();
      return S_OK;
    };

    std::wstring trimmed = reading;
    TrimWstringInPlace(trimmed);
    std::array<uint32_t, 64> bounds{};
    const size_t n = SrfTip_SyllableBoundaryOffsetsUtf16(trimmed.c_str(), trimmed.size(), bounds.data(),
                                                         bounds.size());
    if (n >= 3 && bounds[1] < trimmed.size()) {
      const std::wstring firstSyl = trimmed.substr(0, bounds[1]);
      std::wstring restReading = trimmed.substr(bounds[1]);
      TrimWstringInPlace(restReading);
      if (!firstSyl.empty() && !restReading.empty()) {
        return continueSingleCharCommit(std::move(restReading), &firstSyl);
      }
    }

    if (n == 0) {
      // The engine can temporarily fail to provide boundaries while warming up or under lock
      // contention. Never guess by removing one letter: "nihao" would become the invalid
      // remainder "ihao" after committing "ni"'s character.
      SrfTsfDiagnosticLog(L"commit-candidate.single-char-continuation",
                          L"status=skipped reason=syllable_bounds_unavailable");
    }
  }
  if (committed.size() == 1 && pic && clipboardGameContinuation) {
    SrfTsfDiagnosticLog(L"commit-candidate.single-char-continuation",
                        L"status=skipped reason=clipboard_transport");
  }

  // 逐字续组词路径会在上面提前返回并保留候选 UI；其余提交都结束候选会话。
  if (m_candidateUi) {
    const ULONGLONG uiHideStart = GetTickCount64();
    m_candidateUi->End();
    DebugLogPerfMs(L"CommitCandidate/ui-hide", uiHideStart);
  }

  if (commitAsFunctionKey) {
    if (m_pComposition) {
      CancelCompositionEdit(ec);
    } else if (usedComposition) {
      ClearCompositionBufferState();
    } else {
      ReleaseCompositionState();
    }
    if (SendVirtualKeyTap(committedFunctionVk)) {
      DebugLogPerfMs(L"CommitCandidate/function-key", commitStart);
      SrfTsfDiagnosticLog(L"commit-candidate.end", L"status=ok path=function_key");
      return S_OK;
    }
    const DWORD err = GetLastError();
    return HRESULT_FROM_WIN32(err != 0 ? err : ERROR_GEN_FAILURE);
  }

  HRESULT hr = E_FAIL;
  const ULONGLONG textWriteStart = GetTickCount64();
  const bool finishUserPhraseByDirectInsert = m_userPhraseComposeActive && pic != nullptr;
  if (EffectiveCommitTransport() != SrfCommitTransport::Tsf) {
    ITfContext* ctx = m_pCompositionContext ? m_pCompositionContext : (pic ? pic : m_pFocusContext);
    if (ctx) {
      ctx->AddRef();
      hr = CommitDirectText(ec, ctx, committed);
      ctx->Release();
    } else {
      hr = E_FAIL;
    }
  } else if (finishUserPhraseByDirectInsert) {
    ITfContext* ctx = m_pCompositionContext ? m_pCompositionContext : pic;
    ctx->AddRef();

    if (m_pComposition) {
      const SrfPhraseComposeStateGuard phraseComposeGuard(this);
      ITfRange* range = nullptr;
      if (SUCCEEDED(m_pComposition->GetRange(&range)) && range) {
        (void)range->SetText(ec, 0, L"", 0);
        (void)CollapseSelectionToRangeEnd(ec, ctx, range);
        range->Release();
      }
      (void)m_pComposition->EndComposition(ec);
      ClearCompositionBufferState();
      ReleaseCompositionObjects();
    }

    hr = CommitDirectText(ec, ctx, committed);
    ctx->Release();
  } else if (m_pComposition) {
    ITfRange* range = nullptr;
    hr = m_pComposition->GetRange(&range);
    if (SUCCEEDED(hr) && range) {
      hr = range->SetText(ec, 0, committed.c_str(), static_cast<LONG>(committed.size()));
      if (SUCCEEDED(hr)) {
        ITfContext* commitContext = m_pCompositionContext ? m_pCompositionContext : pic;
        (void)CollapseSelectionToRangeEnd(ec, commitContext, range);
      }
      range->Release();
    }
    if (SUCCEEDED(hr)) hr = m_pComposition->EndComposition(ec);
  } else if (m_pCompositionContext) {
    hr = CommitDirectText(ec, m_pCompositionContext, committed);
  } else if (m_pFocusContext) {
    hr = CommitDirectText(ec, m_pFocusContext, committed);
  }
  DebugLogPerfMs(L"CommitCandidate/text-write", textWriteStart);

  if (FAILED(hr) && EffectiveCommitTransport() != SrfCommitTransport::UnicodeSendInput) {
    bool appGameProfile = false;
    const SrfAppOptions* appOptions = FindAppOptions(m_config, CompatibilityAppName());
    if (appOptions && appOptions->hasGameProfile && appOptions->gameCompactProfile) {
      appGameProfile = true;
    }
    const bool compatibilityFallbackAllowed =
        appGameProfile || m_gameCompatActive || m_configuredGameCompatActive ||
        m_builtinGameCompatActive || m_fullscreenCompatActive || m_manualGameCompatActive;
    if (compatibilityFallbackAllowed) {
      if (m_pComposition) {
        CancelCompositionEdit(ec);
        ReleaseCompositionObjects();
      }
      const HRESULT fallbackHr = SendUnicodeTextInput(committed);
      std::wstring line = L"from=";
      line += EffectiveCommitTransportName();
      line += L", to=unicode_sendinput, status=";
      line += SUCCEEDED(fallbackHr) ? L"ok" : L"failed";
      line += L", len=";
      line += std::to_wstring(committed.size());
      if (FAILED(fallbackHr)) {
        wchar_t hrBuf[16] = {};
        swprintf_s(hrBuf, L"%08lX", static_cast<unsigned long>(fallbackHr));
        line += L", hr=0x";
        line += hrBuf;
      }
      SrfTsfDiagnosticLog(L"commit-transport-fallback", line.c_str());
      if (SUCCEEDED(fallbackHr)) hr = S_OK;
    }
  }

  if (SUCCEEDED(hr) && !suppressCandidateLearning && !reading.empty() && committed != reading) {
    if (committedMeta.correctionCandidate && !committedMeta.correctedReading.empty()) {
      SrfTip_LearnCorrection(reading, committedMeta.correctedReading, committed);
    } else {
      SrfTip_LearnCommitEx(
          reading, committed,
          explicitSelection ? kSrfLearnCommitExplicitSelection : kSrfLearnCommitDefault);
    }
  } else if (SUCCEEDED(hr) && suppressCandidateLearning) {
    SrfTip_ResetLearningContext();
  }

  // 用户选择候选：把强跳过项和当前页弱未选项回传为用户词交互反馈。
  if (SUCCEEDED(hr) && !suppressCandidateLearning && !reading.empty() && committed != reading
      && idx != static_cast<size_t>(-1)) {
    const std::vector<std::wstring> emptySkippedCandidates;
    const std::vector<std::wstring>& skippedCandidates =
        snapshotSkippedCandidates ? *snapshotSkippedCandidates : emptySkippedCandidates;
    if (idx > 0 || !skippedCandidates.empty()) {
      SrfTip_LearnSelectionFeedback(reading, committed, static_cast<unsigned long>(idx),
                                    m_candPage, skippedCandidates);
    }
  }

  // 若刚才处于逐字拼词模式，则在结束上屏时把整词写入用户词典。
  // 触发条件：逐字路径已累计过，且这次不是继续保留剩余 reading 的路径（到这里意味着 composition 将结束）。
  if (SUCCEEDED(hr) && m_userPhraseComposeActive) {
    // 把本次上屏也纳入最终词组（最后一步可能是单字或直接选中某个短语）。
    if (!suppressPhraseComposeLearning && !committed.empty()) {
      m_userPhraseComposeCommitted += committed;
      if (m_userPhraseComposeCommitted.size() > 8) m_userPhraseComposeValid = false;
    }
    // 仅对 2 字及以上的“词组”做学习，避免覆盖单字学习策略。
    if (!suppressPhraseComposeLearning && m_userPhraseComposeValid &&
        m_userPhraseComposeCommitted.size() >= 2 && m_userPhraseComposeCommitted.size() <= 8 &&
        !m_userPhraseComposeOriginalReading.empty()) {
      if (EnsureDeferredTimerWindow()) {
        const unsigned long long requestId = SrfTip_LearnCommitExWithCompletion(
            m_userPhraseComposeOriginalReading, m_userPhraseComposeCommitted,
            kSrfLearnCommitComposedPhrase, m_deferredTimerHwnd,
            kLearnCommitCompletedMessage);
        if (requestId != 0) {
          if (m_pendingLearnNotifications.size() >= 64) {
            m_pendingLearnNotifications.erase(m_pendingLearnNotifications.begin());
          }
          PendingLearnNotification pending;
          pending.requestId = requestId;
          pending.reading = m_userPhraseComposeOriginalReading;
          pending.phrase = m_userPhraseComposeCommitted;
          m_pendingLearnNotifications.push_back(std::move(pending));
        }
      } else {
        // Learning must not depend on notification infrastructure being
        // available; it simply remains silent when no completion window exists.
        SrfTip_LearnCommitEx(m_userPhraseComposeOriginalReading, m_userPhraseComposeCommitted,
                             kSrfLearnCommitComposedPhrase);
      }
    }
    m_userPhraseComposeActive = false;
    m_userPhraseComposeValid = false;
    m_userPhraseComposeOriginalReading.clear();
    m_userPhraseComposeCommitted.clear();
    ApplyRustModeFlags();
  }

  if (FAILED(hr)) {
    RecordCompatibilityFallback(L"CommitCandidate", hr);
    wchar_t hrBuf[16] = {};
    swprintf_s(hrBuf, L"%08lX", static_cast<unsigned long>(hr));
    std::wstring line = L"status=failed, hr=0x";
    line += hrBuf;
    line += L", total_ms=";
    line += std::to_wstring(GetTickCount64() - commitStart);
    SrfTsfDiagnosticLog(L"commit-candidate.end", line.c_str());
    return hr;
  }

  if (usedComposition) {
    ClearCompositionBufferState();
  } else {
    ReleaseCompositionState();
  }
  std::wstring line = L"status=ok, total_ms=";
  line += std::to_wstring(GetTickCount64() - commitStart);
  line += L", committed_len=";
  line += std::to_wstring(committed.size());
  SrfTsfDiagnosticLog(L"commit-candidate.end", line.c_str());
  return hr;
}

void CSrfTip::OnCandidateClicked(UINT indexInPage) {
  if (m_candidateUi) m_candidateUi->OnCandidateClicked(indexInPage);
}

void CSrfTip::OnCandidateWheel(int wheelDelta) {
  if (m_candidateUi) m_candidateUi->OnCandidateWheel(wheelDelta);
}

namespace {

std::wstring CandidatePinStateKey(const std::wstring& reading, const std::wstring& candidate) {
  std::wstring key = reading;
  key.push_back(L'\t');
  key += candidate;
  return key;
}

}  // namespace

bool CSrfTip::IsCandidatePinned(size_t idx) const {
  if (idx >= m_candidates.size() || m_reading.empty()) return false;
  const std::wstring key = CandidatePinStateKey(m_reading, m_candidates[idx]);
  if (m_locallyPinnedCandidateKeys.find(key) != m_locallyPinnedCandidateKeys.end()) return true;
  if (idx < m_candidateMeta.size() && !m_candidateMeta[idx].empty()) {
    return SplitCandidateMeta(m_candidateMeta[idx]).pinnedExactInput;
  }
  return false;
}

std::wstring CSrfTip::CandidateSourceDescription(size_t idx) const {
  if (idx >= m_candidates.size()) return L"";
  std::vector<std::wstring> parts;
  if (IsCandidatePinned(idx)) parts.push_back(L"\u7528\u6237\u7f6e\u9876");
  if (idx < m_candidateMeta.size() && !m_candidateMeta[idx].empty()) {
    const CandidateMetaParts meta = SplitCandidateMeta(m_candidateMeta[idx]);
    if (meta.userCandidate) {
      parts.push_back(meta.userSourceLabel.empty() ? L"\u7528\u6237\u5e38\u7528"
                                                   : meta.userSourceLabel);
    }
    if (meta.extCandidate) parts.push_back(L"ext/large");
    if (meta.correctionCandidate) parts.push_back(L"\u7ea0\u9519");
    if (meta.noLearn) parts.push_back(L"\u4e0d\u5b66\u4e60");
    if (meta.partialResult) parts.push_back(L"\u90e8\u5206\u7ed3\u679c");
  }
  if (parts.empty()) parts.push_back(L"\u7cfb\u7edf\u8bcd\u5e93");

  std::wstring text = L"\u6765\u6e90\uff1a";
  for (size_t i = 0; i < parts.size(); ++i) {
    if (i > 0) text += L" / ";
    text += parts[i];
  }
  return text;
}

void CSrfTip::ToggleCandidatePinChoice(size_t idx) {
  ApplyCandidatePinChoice(idx, !IsCandidatePinned(idx));
}

void CSrfTip::ApplyCandidateMenuCommand(size_t idx, int command) {
  if (idx >= m_candidates.size() || m_reading.empty()) return;
  if (m_candidatesReading != m_reading) return;

  switch (command) {
    case kSrfCandidateWindowMenuPin:
      ApplyCandidatePinChoice(idx, true);
      return;
    case kSrfCandidateWindowMenuUnpin:
      ApplyCandidatePinChoice(idx, false);
      return;
    case kSrfCandidateWindowMenuSource: {
      if (m_config.ShouldShowNotification(SrfNotificationKind::AppOptions)) {
        ShowNotification(SrfNotificationKind::AppOptions, CandidateSourceDescription(idx));
      }
      return;
    }
    case kSrfCandidateWindowMenuRemoveUserPhrase:
    case kSrfCandidateWindowMenuBlockPhrase:
      break;
    default:
      return;
  }

  if (m_traditionalOutput) {
    if (m_config.ShouldShowNotification(SrfNotificationKind::AppOptions)) {
      ShowNotification(SrfNotificationKind::AppOptions,
                       L"\u7e41\u9ad4\u6a21\u5f0f\u4e0d\u5199\u5165\u7528\u6237\u8bcd\u5e93");
    }
    return;
  }
  if (ShouldSuppressLearningForPrivacy()) {
    if (m_config.ShouldShowNotification(SrfNotificationKind::AppOptions)) {
      ShowNotification(SrfNotificationKind::AppOptions,
                       L"\u5f53\u524d\u5e94\u7528\u5df2\u8bbe\u4e3a\u6c38\u4e0d\u5b66\u4e60");
    }
    return;
  }

  const std::wstring reading = m_reading;
  const std::wstring candidate = m_candidates[idx];
  const bool remove = command == kSrfCandidateWindowMenuRemoveUserPhrase;
  SrfTip_ApplyCandidateAction(reading, candidate,
                              remove ? kSrfCandidateActionRemoveUserPhrase
                                     : kSrfCandidateActionBlockPhrase);

  const std::wstring pinKey = CandidatePinStateKey(reading, candidate);
  m_locallyPinnedCandidateKeys.erase(pinKey);
  m_candSel = 0;
  m_candPage = 0;
  RefreshCandidatesAsync();
  ClampCandidateState();
  SyncCandidateContextState();
  RedrawCandidateUi();

  if (m_config.ShouldShowNotification(SrfNotificationKind::AppOptions)) {
    ShowNotification(SrfNotificationKind::AppOptions,
                     remove ? L"\u5df2\u5c1d\u8bd5\u5220\u9664\u7528\u6237\u8bcd"
                            : L"\u5df2\u5c4f\u853d\u8be5\u8bcd");
  }
}

void CSrfTip::ApplyCandidatePinChoice(size_t idx, bool pinned) {
  if (idx >= m_candidates.size() || m_reading.empty()) return;
  if (m_candidatesReading != m_reading) return;
  if (m_traditionalOutput) {
    if (m_config.ShouldShowNotification(SrfNotificationKind::AppOptions)) {
      ShowNotification(SrfNotificationKind::AppOptions,
                       L"\u7e41\u9ad4\u6a21\u5f0f\u4e0d\u5199\u5165\u7528\u6237\u8bcd\u5e93");
    }
    return;
  }
  if (ShouldSuppressLearningForPrivacy()) {
    if (m_config.ShouldShowNotification(SrfNotificationKind::AppOptions)) {
      ShowNotification(SrfNotificationKind::AppOptions,
                       L"\u5f53\u524d\u5e94\u7528\u5df2\u8bbe\u4e3a\u6c38\u4e0d\u5b66\u4e60");
    }
    return;
  }

  const std::wstring reading = m_reading;
  const std::wstring candidate = m_candidates[idx];
  std::wstring line = L"idx=";
  line += std::to_wstring(idx);
  line += L", pinned=";
  line += pinned ? L"1" : L"0";
  line += L", reading=";
  line += ShortenForLog(reading, 24);
  line += L", candidate=";
  line += ShortenForLog(candidate, 24);
  SrfTsfDiagnosticLog(L"candidate-pin-menu.apply", line.c_str());
  if (!SrfTip_SetCandidatePin(reading, candidate, pinned)) {
    SrfTsfDiagnosticLog(L"candidate-pin-menu.apply-failed",
                        (L"idx=" + std::to_wstring(idx) + L" pinned=" +
                         (pinned ? L"1" : L"0") + L" reason=ipc-failed").c_str());
    return;
  }

  const std::wstring pinKey = CandidatePinStateKey(reading, candidate);
  if (pinned) {
    m_locallyPinnedCandidateKeys.insert(pinKey);
  } else {
    m_locallyPinnedCandidateKeys.erase(pinKey);
  }

  if (pinned) {
    m_candSel = 0;
    m_candPage = 0;
  }
  RefreshCandidatesAsync();
  ClampCandidateState();
  SyncCandidateContextState();
  RedrawCandidateUi();

  if (m_config.ShouldShowNotification(SrfNotificationKind::AppOptions)) {
    ShowNotification(SrfNotificationKind::AppOptions,
                     pinned ? L"\u5df2\u56fa\u5b9a\u7f6e\u9876" : L"\u5df2\u53d6\u6d88\u56fa\u5b9a");
  }
}
