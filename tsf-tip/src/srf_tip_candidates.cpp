#include "srf_tip_detail.h"

using namespace srf_tip_detail;

namespace {

bool LookupStatusNeedsCandidateBackpressure(SrfLookupCandidatesStatus status) {
  switch (status) {
    case SrfLookupCandidatesStatus::BridgeBusy:
    case SrfLookupCandidatesStatus::Superseded:
    case SrfLookupCandidatesStatus::RemoteBusy:
      return true;
    case SrfLookupCandidatesStatus::Ok:
    case SrfLookupCandidatesStatus::Empty:
    case SrfLookupCandidatesStatus::EngineNotReady:
    case SrfLookupCandidatesStatus::EnsureFailed:
    case SrfLookupCandidatesStatus::TransientFailure:
    case SrfLookupCandidatesStatus::Failed:
    case SrfLookupCandidatesStatus::BackendNotConnected:
      return false;
  }
  return false;
}

}  // namespace

void CSrfTip::RefreshCandidates() {
  CancelDeferredCandidateRefresh();
  ResetCandidateFullLookupState();
  const unsigned long long engineRequestId = SrfTip_NextLookupRequestId();
  SrfTip_CancelPendingLookupBefore(engineRequestId);
  if (ShouldSuppressCandidatesForPrivacy()) {
    m_candidateView.candidates.clear();
    m_candidateView.candidateMeta.clear();
    m_candidateView.candidateRows.clear();
    m_candidateView.candidatesReading.clear();
    m_candidateView.candidateHasMore = false;
    SetCandidateViewState(SrfCandidateViewState::Empty, L"privacy");
    InvalidateCandidatePageLayoutCache();
    ClampCandidateState();
    RebuildContextModel();
    SyncStatusModel();
    RedrawCandidateUi();
    SrfTsfDiagnosticLog(L"candidate-refresh.skip", L"reason=privacy_disabled");
    return;
  }
  const SrfEngineState stateBefore = SrfTip_GetEngineState();
  const unsigned long long requestId =
      !m_compositionState.reading.empty()
          ? m_candidateLookupSerial.fetch_add(1, std::memory_order_acq_rel) + 1
          : 0;
  if (!m_compositionState.reading.empty()) {
    std::wstring beginLine = L"request_id=" + std::to_wstring(requestId);
    beginLine += L", reading=";
    beginLine += ShortenForLog(m_compositionState.reading);
    beginLine += L", engine=";
    beginLine += EngineStateName(stateBefore);
    SrfTsfPerfLog(L"candidate-refresh.begin", beginLine.c_str());
  }
  std::vector<std::wstring> nextCandidates;
  std::vector<CandidateMetaParts> nextMeta;
  bool hasMore = false;
  const SrfLookupCandidatesStatus lookupStatus = SrfTip_LookupCandidates(
      m_compositionState.reading, nextCandidates, &nextMeta, engineRequestId, false, &hasMore);
  MaybeInsertPredictedPhraseCandidates(m_compositionState.reading, &nextCandidates, &nextMeta);
  const SrfEngineState stateAfterLookup = SrfTip_GetEngineState();
  ApplyCandidateRefreshResult(m_compositionState.reading, std::move(nextCandidates),
                              std::move(nextMeta), stateAfterLookup, lookupStatus, false, requestId,
                              hasMore, false);
}

bool CSrfTip::EnsureBlindCommitCandidatesReady() {
  if (!ShouldUseBlindCommitForCompatibility()) return false;
  if (m_compositionState.reading.empty()) return false;
  if (!m_candidateView.candidates.empty() &&
      m_candidateView.candidatesReading == m_compositionState.reading &&
      !CurrentCandidatesPartial()) {
    return true;
  }

  const SrfEngineState stateBefore = SrfTip_GetEngineState();
  if (stateBefore != SrfEngineState::Ready && m_candidateView.candidates.empty()) {
    std::wstring line = L"reading=";
    line += ShortenForLog(m_compositionState.reading, 24);
    line += L", engine=";
    line += EngineStateName(stateBefore);
    line += L", ready=0, reason=engine_not_ready";
    SrfTsfDiagnosticLog(L"blind-commit.skip", line.c_str());
    return false;
  }

  const ULONGLONG start = GetTickCount64();
  const std::wstring reading = m_compositionState.reading;
  RefreshCandidates();
  const bool ready = !m_candidateView.candidates.empty() &&
                     m_candidateView.candidatesReading == reading &&
                     m_compositionState.reading == reading;

  std::wstring line = L"reading=";
  line += ShortenForLog(reading, 24);
  line += L", ready=";
  line += ready ? L"1" : L"0";
  line += L", count=";
  line += std::to_wstring(m_candidateView.candidates.size());
  line += L", elapsed_ms=";
  line += std::to_wstring(GetTickCount64() - start);
  SrfTsfDiagnosticLog(L"blind-commit.refresh", line.c_str());
  return ready;
}

bool CSrfTip::EnsureCandidateLookupWorker() {
  if (m_candidateWorkerThread.joinable()) return true;
  {
    std::lock_guard<std::mutex> guard(m_candidateWorkerMutex);
    m_candidateWorkerStop = false;
    m_candidateWorkerHasRequest = false;
    m_candidateWorkerRequestReading.clear();
    m_candidateWorkerRequestSerial = 0;
    m_candidateWorkerEngineRequestId = 0;
    m_candidateWorkerRequestFullResult = false;
    m_candidateWorkerNotifyHwnd = nullptr;
    m_candidateWorkerRequestTick = 0;
    m_candidateWorkerRapidRequestCount = 0;
    m_candidateWorkerRequestFocus = {};
    m_candidateLookupBackpressureUntilTick.store(0, std::memory_order_release);
  }
  SrfTip_BackgroundWorkerAddRef();
  try {
    m_candidateWorkerThread = std::thread([this]() { CandidateLookupWorkerMain(); });
  } catch (...) {
    SrfTip_BackgroundWorkerRelease();
    SrfTsfDiagnosticLog(L"candidate-worker.start-failed", L"thread create failed");
    return false;
  }
  SrfTsfDiagnosticLog(L"candidate-worker.start", L"started");
  return true;
}

void CSrfTip::StopCandidateLookupWorker() {
  if (!m_candidateWorkerThread.joinable()) return;
  {
    std::lock_guard<std::mutex> guard(m_candidateWorkerMutex);
    m_candidateWorkerStop = true;
    m_candidateWorkerHasRequest = false;
    m_candidateWorkerRequestReading.clear();
    m_candidateWorkerRequestSerial = 0;
    m_candidateWorkerEngineRequestId = 0;
    m_candidateWorkerRequestFullResult = false;
    m_candidateWorkerNotifyHwnd = nullptr;
    m_candidateWorkerRequestTick = 0;
    m_candidateWorkerRapidRequestCount = 0;
    m_candidateWorkerRequestFocus = {};
    m_candidateLookupBackpressureUntilTick.store(0, std::memory_order_release);
  }
  m_candidateWorkerCv.notify_all();
  if (m_candidateWorkerThread.get_id() != std::this_thread::get_id()) {
    m_candidateWorkerThread.join();
    SrfTip_BackgroundWorkerRelease();
    SrfTsfDiagnosticLog(L"candidate-worker.stop", L"stopped");
  }
}

void CSrfTip::QueueCandidateLookup(const std::wstring& reading, unsigned long long serial,
                                   unsigned long long engineRequestId, HWND hwnd,
                                   const SrfFocusSnapshot& focus, bool fullResult) {
  {
    std::lock_guard<std::mutex> guard(m_candidateWorkerMutex);
    if (m_candidateWorkerStop) return;
    const ULONGLONG now = GetTickCount64();
    if (m_candidateWorkerRequestTick != 0 && now >= m_candidateWorkerRequestTick &&
        now - m_candidateWorkerRequestTick <= kCandidateLookupRapidInputMs) {
      m_candidateWorkerRapidRequestCount =
          std::min<unsigned int>(m_candidateWorkerRapidRequestCount + 1, 8);
    } else {
      m_candidateWorkerRapidRequestCount = 0;
    }
    m_candidateWorkerRequestReading = reading;
    m_candidateWorkerRequestSerial = serial;
    m_candidateWorkerEngineRequestId = engineRequestId;
    m_candidateWorkerRequestFullResult = fullResult;
    m_candidateWorkerNotifyHwnd = hwnd;
    m_candidateWorkerRequestTick = now;
    m_candidateWorkerRequestFocus = focus;
    m_candidateWorkerHasRequest = true;
  }
  m_candidateWorkerCv.notify_one();
}

void CSrfTip::CandidateLookupWorkerMain() {
  for (;;) {
    std::wstring reading;
    unsigned long long serial = 0;
    unsigned long long engineRequestId = 0;
    HWND hwnd = nullptr;
    ULONGLONG requestTick = 0;
    SrfFocusSnapshot focus = {};
    bool fullResult = false;
    {
      std::unique_lock<std::mutex> lock(m_candidateWorkerMutex);
      m_candidateWorkerCv.wait(
          lock, [&]() { return m_candidateWorkerStop || m_candidateWorkerHasRequest; });
      if (m_candidateWorkerStop) break;

      auto coalesceMs = [&]() -> DWORD {
        const ULONGLONG now = GetTickCount64();
        const ULONGLONG backpressureUntil =
            m_candidateLookupBackpressureUntilTick.load(std::memory_order_acquire);
        if (backpressureUntil != 0 && now < backpressureUntil) {
          return kCandidateLookupBackpressureCoalesceMs;
        }
        if (m_candidateWorkerRapidRequestCount >= 2) return kCandidateLookupRapidCoalesceMs;
        if (m_candidateWorkerRapidRequestCount == 1) return kCandidateLookupWarmCoalesceMs;
        return kCandidateLookupCoalesceMs;
      };
      const auto coalesceStarted = std::chrono::steady_clock::now();
      for (;;) {
        const unsigned long long observedSerial = m_candidateWorkerRequestSerial;
        // Bound a burst by its first wait: repeated updates must not restart
        // the entire debounce interval and postpone useful work indefinitely.
        const auto deadline = coalesceStarted + std::chrono::milliseconds(coalesceMs());
        const bool wokeForUpdate = m_candidateWorkerCv.wait_until(lock, deadline, [&]() {
          return m_candidateWorkerStop || m_candidateWorkerRequestSerial != observedSerial;
        });
        if (m_candidateWorkerStop || !wokeForUpdate) break;
      }
      if (m_candidateWorkerStop) break;

      reading = m_candidateWorkerRequestReading;
      serial = m_candidateWorkerRequestSerial;
      engineRequestId = m_candidateWorkerEngineRequestId;
      fullResult = m_candidateWorkerRequestFullResult;
      hwnd = m_candidateWorkerNotifyHwnd;
      requestTick = m_candidateWorkerRequestTick;
      focus = m_candidateWorkerRequestFocus;
      m_candidateWorkerHasRequest = false;
      m_candidateWorkerRequestReading.clear();
      m_candidateWorkerRequestSerial = 0;
      m_candidateWorkerEngineRequestId = 0;
      m_candidateWorkerRequestFullResult = false;
      m_candidateWorkerNotifyHwnd = nullptr;
      m_candidateWorkerRequestFocus = {};
    }

    if (reading.empty() || !hwnd) continue;
    const unsigned long long currentBeforeLookup =
        m_candidateLookupSerial.load(std::memory_order_acquire);
    if (serial != currentBeforeLookup) {
      std::wstring line = L"reading_units=" + std::to_wstring(reading.size());
      line += L", request_id=";
      line += std::to_wstring(serial);
      line += L", currentserial=";
      line += std::to_wstring(currentBeforeLookup);
      SrfTsfPerfLog(L"candidate-worker.drop-before", line.c_str());
      continue;
    }

    std::vector<std::wstring> candidates;
    std::vector<CandidateMetaParts> meta;
    bool hasMore = false;
    SrfEngineState stateAfterLookup = SrfEngineState::Idle;
    SrfLookupCandidatesStatus lookupStatus = SrfLookupCandidatesStatus::Failed;
    const ULONGLONG lookupStart = GetTickCount64();
    if (requestTick != 0) DebugLogPerfMs(L"CandidateWorker/queue-wait", requestTick);
    try {
      lookupStatus = SrfTip_LookupCandidates(reading, candidates, &meta, engineRequestId,
                                             fullResult, &hasMore);
      MaybeInsertPredictedPhraseCandidates(reading, &candidates, &meta);
      stateAfterLookup = SrfTip_GetEngineState();
    } catch (...) {
      candidates.clear();
      meta.clear();
      stateAfterLookup = SrfTip_GetEngineState();
      lookupStatus = SrfLookupCandidatesStatus::Failed;
    }
    if (LookupStatusNeedsCandidateBackpressure(lookupStatus)) {
      m_candidateLookupBackpressureUntilTick.store(
          GetTickCount64() + kCandidateLookupBackpressureMs, std::memory_order_release);
    } else {
      m_candidateLookupBackpressureUntilTick.store(0, std::memory_order_release);
    }
    DebugLogPerfMs(L"CandidateWorker/lookup", lookupStart);

    if (serial == m_candidateLookupSerial.load(std::memory_order_acquire)) {
      {
        std::lock_guard<std::mutex> guard(m_asyncCandidateMutex);
        m_asyncCandidatePending = true;
        m_asyncCandidatePendingSerial = serial;
        m_asyncCandidatePendingReading = reading;
        m_asyncCandidatePendingItems = std::move(candidates);
        m_asyncCandidatePendingMeta = std::move(meta);
        m_asyncCandidatePendingEngineState = stateAfterLookup;
        m_asyncCandidatePendingLookupStatus = lookupStatus;
        m_asyncCandidatePendingHasMore = hasMore;
        m_asyncCandidatePendingFullResult = fullResult;
        m_asyncCandidatePendingRequestTick = requestTick;
        m_asyncCandidatePendingFocus = focus;
      }
      PostMessageW(hwnd, kAsyncCandidateResultMessage, 0, 0);
    } else {
      m_candidateLookupBackpressureUntilTick.store(
          GetTickCount64() + kCandidateLookupBackpressureMs, std::memory_order_release);
      std::wstring line = L"reading_units=" + std::to_wstring(reading.size());
      line += L", request_id=";
      line += std::to_wstring(serial);
      line += L", currentserial=";
      line += std::to_wstring(m_candidateLookupSerial.load(std::memory_order_acquire));
      SrfTsfPerfLog(L"candidate-worker.drop-after", line.c_str());
    }
  }
}

bool CSrfTip::TryApplyPrefixCandidatePlaceholder(const std::wstring& reading,
                                                 SrfEngineState stateBefore,
                                                 unsigned long long requestId) {
  if (reading.size() < 3 || m_prefixCandidateCache.empty()) return false;
  const ULONGLONG now = GetTickCount64();
  size_t bestIndex = static_cast<size_t>(-1);
  size_t bestPrefixLen = 0;
  for (size_t i = 0; i < m_prefixCandidateCache.size(); ++i) {
    const auto& entry = m_prefixCandidateCache[i];
    if (entry.reading.empty() || entry.candidates.empty()) continue;
    if (entry.reading.size() >= reading.size()) continue;
    if (now >= entry.tick && now - entry.tick > kPrefixCandidateCacheTtlMs) continue;
    if (reading.rfind(entry.reading, 0) != 0) continue;
    if (entry.reading.size() > bestPrefixLen) {
      bestPrefixLen = entry.reading.size();
      bestIndex = i;
    }
  }
  if (bestIndex == static_cast<size_t>(-1)) return false;

  const auto& entry = m_prefixCandidateCache[bestIndex];
  std::vector<std::wstring> placeholderCandidates = entry.candidates;
  std::vector<CandidateMetaParts> placeholderMeta = entry.meta;
  const size_t limit =
      std::min(placeholderCandidates.size(), static_cast<size_t>(CandidatePageSize()));
  if (placeholderCandidates.size() > limit) placeholderCandidates.resize(limit);
  placeholderMeta.resize(placeholderCandidates.size());
  for (auto& meta : placeholderMeta) {
    meta.noLearn = true;
    meta.partialResult = true;
    meta.prefixPlaceholder = true;
  }

  std::wstring line = L"request_id=" + std::to_wstring(requestId);
  line += L", reading=";
  line += ShortenForLog(reading, 24);
  line += L", prefix=";
  line += ShortenForLog(entry.reading, 24);
  line += L", count=";
  line += std::to_wstring(placeholderCandidates.size());
  SrfTsfPerfLog(L"candidate-refresh.prefix-placeholder", line.c_str());

  return ApplyCandidateRefreshResult(reading, std::move(placeholderCandidates),
                                     std::move(placeholderMeta), stateBefore,
                                     SrfLookupCandidatesStatus::Ok, false, requestId);
}

void CSrfTip::RememberPrefixCandidateCache(const std::wstring& reading) {
  if (reading.size() < 2 || m_candidateView.candidates.empty() ||
      m_candidateView.candidatesReading != reading)
    return;
  for (wchar_t ch : reading) {
    if ((ch >= L'a' && ch <= L'z') || (ch >= L'A' && ch <= L'Z') || ch == L'\'') continue;
    return;
  }
  for (const auto& meta : m_candidateView.candidateMeta) {
    if (meta.prefixPlaceholder) return;
  }

  for (auto it = m_prefixCandidateCache.begin(); it != m_prefixCandidateCache.end(); ++it) {
    if (it->reading == reading) {
      m_prefixCandidateCache.erase(it);
      break;
    }
  }
  PrefixCandidateCacheEntry entry;
  entry.reading = reading;
  entry.candidates = m_candidateView.candidates;
  entry.meta = m_candidateView.candidateMeta;
  entry.tick = GetTickCount64();
  m_prefixCandidateCache.push_back(std::move(entry));
  while (m_prefixCandidateCache.size() > kPrefixCandidateCacheCapacity) {
    m_prefixCandidateCache.erase(m_prefixCandidateCache.begin());
  }
}

void CSrfTip::RebuildCandidateRows() {
  m_candidateView.candidateRows.clear();
  m_candidateView.candidateRows.reserve(m_candidateView.candidates.size());
  for (size_t i = 0; i < m_candidateView.candidates.size(); ++i) {
    CandidateRow row;
    row.text = m_candidateView.candidates[i];
    if (i < m_candidateView.candidateMeta.size()) {
      row.meta = m_candidateView.candidateMeta[i];
    }
    m_candidateView.candidateRows.push_back(std::move(row));
  }
}

bool CSrfTip::RefreshCandidatesAsync(bool fullResult) {
  CancelDeferredCandidateRefresh();
  const unsigned long long engineRequestId = SrfTip_NextLookupRequestId();
  SrfTip_CancelPendingLookupBefore(engineRequestId);
  if (m_compositionState.reading.empty()) {
    ResetCandidateFullLookupState();
    SetCandidateViewState(SrfCandidateViewState::Empty, L"async-empty-reading");
    return false;
  }
  if (ShouldSuppressCandidatesForPrivacy()) {
    m_candidateView.candidates.clear();
    m_candidateView.candidateMeta.clear();
    m_candidateView.candidateRows.clear();
    m_candidateView.candidatesReading.clear();
    m_candidateView.candidateHasMore = false;
    ResetCandidateFullLookupState();
    SetCandidateViewState(SrfCandidateViewState::Empty, L"async-privacy");
    InvalidateCandidatePageLayoutCache();
    ClampCandidateState();
    RebuildContextModel();
    SyncStatusModel();
    RedrawCandidateUi();
    SrfTsfDiagnosticLog(L"candidate-refresh.async-skip", L"reason=privacy_disabled");
    return false;
  }

  const std::wstring reading = m_compositionState.reading;
  const unsigned long long serial =
      m_candidateLookupSerial.fetch_add(1, std::memory_order_acq_rel) + 1;
  if (fullResult) {
    m_candidateView.candidateFullLookupPending = true;
    m_candidateView.candidateFullLookupSerial = serial;
  } else {
    ResetCandidateFullLookupState();
  }
  const SrfEngineState stateBefore = SrfTip_GetEngineState();
  ITfContext* context = m_compositionState.pCompositionContext
                            ? m_compositionState.pCompositionContext
                            : m_pFocusContext;
  const SrfFocusSnapshot focus = CaptureFocusSnapshot(context);
  InvalidateCandidatePageLayoutCache();

  std::wstring beginLine = L"reading=" + ShortenForLog(reading);
  beginLine += L", request_id=";
  beginLine += std::to_wstring(serial);
  beginLine += L", engine=";
  beginLine += EngineStateName(stateBefore);
  beginLine += L", focus=";
  beginLine += FormatFocusSnapshotForLog(focus);
  SrfTsfPerfLog(L"candidate-refresh.async-begin", beginLine.c_str());

  bool appliedCachedCandidates = false;
  bool skipAsyncLookupAfterCacheHit = false;
  if (!fullResult) {
    std::vector<std::wstring> cachedCandidates;
    std::vector<CandidateMetaParts> cachedMeta;
    bool cachedHasMore = false;
    if (SrfTip_TryGetCachedLookupCandidates(reading, cachedCandidates, &cachedMeta,
                                            &cachedHasMore)) {
      std::wstring cacheLine = L"request_id=" + std::to_wstring(serial);
      cacheLine += L", count=";
      cacheLine += std::to_wstring(cachedCandidates.size());
      SrfTsfPerfLog(L"candidate-refresh.cache-hit", cacheLine.c_str());
      ApplyCandidateRefreshResult(reading, std::move(cachedCandidates), std::move(cachedMeta),
                                  stateBefore, SrfLookupCandidatesStatus::Ok, false, serial,
                                  cachedHasMore, false);
      appliedCachedCandidates = true;
      skipAsyncLookupAfterCacheHit = stateBefore == SrfEngineState::Ready;
    }
  }

  const bool deferCandidateUiUntilAsync =
      !appliedCachedCandidates && stateBefore == SrfEngineState::Ready;
  if (!appliedCachedCandidates) {
    const bool canRetainExistingCandidates = !m_candidateView.candidates.empty();
    if (!canRetainExistingCandidates && stateBefore != SrfEngineState::Ready) {
      appliedCachedCandidates = TryApplyPrefixCandidatePlaceholder(reading, stateBefore, serial);
    } else {
      std::wstring line = L"request_id=" + std::to_wstring(serial);
      line += L", reading=";
      line += ShortenForLog(reading, 24);
      line += L", retained_count=";
      line += std::to_wstring(m_candidateView.candidates.size());
      line += L", candidatesReading=";
      line += ShortenForLog(m_candidateView.candidatesReading, 24);
      if (stateBefore == SrfEngineState::Ready) {
        line += L", reason=engine_ready";
      }
      SrfTsfPerfLog(L"candidate-refresh.prefix-placeholder.skip", line.c_str());
    }
  }

  if (!appliedCachedCandidates) {
    // Keep the previous candidate list visible while the async lookup for the new
    // reading is in flight. Clearing here makes the UI element End()/Begin() on
    // every keystroke, which is perceived as candidate-bar flicker. Commit paths
    // treat a mismatched m_candidateView.candidatesReading as stale, so these retained rows are
    // visual only until the matching result arrives.
    if (!m_candidateView.candidates.empty() && m_candidateView.candidatesReading == reading &&
        !CurrentCandidatesPartial()) {
      SetCandidateViewState(SrfCandidateViewState::Stable, L"async-retain-current");
    } else if (!m_candidateView.candidates.empty() &&
               m_candidateView.candidatesReading != reading) {
      SetCandidateViewState(SrfCandidateViewState::Stale, L"async-retain-stale");
    } else {
      SetCandidateViewState(SrfCandidateViewState::Pending, L"async-pending");
    }
    ClampCandidateState();
    if (!deferCandidateUiUntilAsync) {
      RebuildContextModel();
    }
    SyncStatusModel();
    if (deferCandidateUiUntilAsync) {
      std::wstring line = L"request_id=" + std::to_wstring(serial);
      line += L", reading=";
      line += ShortenForLog(reading, 24);
      line += L", count=";
      line += std::to_wstring(m_candidateView.candidates.size());
      line += L", candidatesReading=";
      line += ShortenForLog(m_candidateView.candidatesReading, 24);
      line += L", reason=await_async_ready";
      SrfTsfPerfLog(L"candidate-refresh.defer-ui", line.c_str());
    }
  }

  if (skipAsyncLookupAfterCacheHit) {
    std::wstring line = L"request_id=" + std::to_wstring(serial);
    line += L", reading_units=";
    line += std::to_wstring(reading.size());
    line += L", reason=cache_hit";
    SrfTsfPerfLog(L"candidate-refresh.cache-skip-async", line.c_str());
    return true;
  }

  if (!EnsureDeferredTimerWindow()) {
    if (fullResult) {
      ResetCandidateFullLookupState();
    }
    std::wstring line = L"request_id=" + std::to_wstring(serial);
    line += L", reason=message_window_unavailable";
    SrfTsfDiagnosticLog(L"candidate-refresh.async-skip", line.c_str());
    return true;
  }
  HWND hwnd = m_deferredTimerHwnd;
  if (!EnsureCandidateLookupWorker()) {
    if (fullResult) {
      ResetCandidateFullLookupState();
    }
    std::wstring line = L"request_id=" + std::to_wstring(serial);
    line += L", reason=worker_unavailable";
    SrfTsfDiagnosticLog(L"candidate-refresh.async-skip", line.c_str());
    return true;
  }
  QueueCandidateLookup(reading, serial, engineRequestId, hwnd, focus, fullResult);
  return !deferCandidateUiUntilAsync;
}

bool CSrfTip::CurrentCandidatesPartial() const {
  for (const auto& row : m_candidateView.candidateRows) {
    if (row.meta.partialResult) return true;
  }
  return false;
}

const wchar_t* CSrfTip::CandidateViewStateName() const {
  switch (m_candidateView.candidateViewState) {
    case SrfCandidateViewState::Empty:
      return L"empty";
    case SrfCandidateViewState::Stable:
      return L"stable";
    case SrfCandidateViewState::Pending:
      return L"pending";
    case SrfCandidateViewState::Stale:
      return L"stale";
    case SrfCandidateViewState::PartialCurrent:
      return L"partial-current";
  }
  return L"unknown";
}

bool CSrfTip::CandidateViewInteractive() const {
  const bool currentInteractive =
      m_candidateView.candidateViewState == SrfCandidateViewState::Stable ||
      m_candidateView.candidateViewState == SrfCandidateViewState::PartialCurrent;
  return currentInteractive && !m_candidateView.candidates.empty() &&
         !m_compositionState.reading.empty() &&
         m_candidateView.candidatesReading == m_compositionState.reading &&
         !ShouldSuppressCandidatesForPrivacy();
}

bool CSrfTip::CandidateViewPendingVisual() const {
  return !m_candidateView.candidates.empty() &&
         (m_candidateView.candidateViewState == SrfCandidateViewState::Pending ||
          m_candidateView.candidateViewState == SrfCandidateViewState::Stale);
}

void CSrfTip::SetCandidateViewState(SrfCandidateViewState state, const wchar_t* reason) {
  if (m_candidateView.candidateViewState == state) return;
  auto stateName = [](SrfCandidateViewState value) -> const wchar_t* {
    switch (value) {
      case SrfCandidateViewState::Empty:
        return L"empty";
      case SrfCandidateViewState::Stable:
        return L"stable";
      case SrfCandidateViewState::Pending:
        return L"pending";
      case SrfCandidateViewState::Stale:
        return L"stale";
      case SrfCandidateViewState::PartialCurrent:
        return L"partial-current";
    }
    return L"unknown";
  };

  const ULONGLONG now = GetTickCount64();
  const ULONGLONG ageMs = m_candidateView.candidateViewStateSinceTick == 0 ||
                                  now < m_candidateView.candidateViewStateSinceTick
                              ? 0
                              : now - m_candidateView.candidateViewStateSinceTick;
  const SrfCandidateViewState oldState = m_candidateView.candidateViewState;
  m_candidateView.candidateViewState = state;
  m_candidateView.candidateViewStateSinceTick = now;

  std::wstring line = L"old=";
  line += stateName(oldState);
  line += L", new=";
  line += stateName(state);
  line += L", reason=";
  line += reason ? reason : L"(none)";
  line += L", old_age_ms=";
  line += std::to_wstring(ageMs);
  line += L", count=";
  line += std::to_wstring(m_candidateView.candidates.size());
  line += L", reading=";
  line += ShortenForLog(m_compositionState.reading, 24);
  line += L", candidatesReading=";
  line += ShortenForLog(m_candidateView.candidatesReading, 24);
  SrfTsfPerfLog(L"candidate-view.state", line.c_str());
}

bool IsRawAlphabeticReadingForCandidateFallback(const std::wstring& reading) {
  if (reading.empty()) return false;
  for (wchar_t ch : reading) {
    if ((ch >= L'a' && ch <= L'z') || (ch >= L'A' && ch <= L'Z') || ch == L'\'') {
      continue;
    }
    return false;
  }
  return true;
}

const wchar_t* LookupCandidatesStatusName(SrfLookupCandidatesStatus status) {
  switch (status) {
    case SrfLookupCandidatesStatus::Ok:
      return L"ok";
    case SrfLookupCandidatesStatus::Empty:
      return L"empty";
    case SrfLookupCandidatesStatus::EngineNotReady:
      return L"engine_not_ready";
    case SrfLookupCandidatesStatus::BridgeBusy:
      return L"bridge_busy";
    case SrfLookupCandidatesStatus::EnsureFailed:
      return L"ensure_failed";
    case SrfLookupCandidatesStatus::Superseded:
      return L"superseded";
    case SrfLookupCandidatesStatus::RemoteBusy:
      return L"remote_busy";
    case SrfLookupCandidatesStatus::TransientFailure:
      return L"transient_failure";
    case SrfLookupCandidatesStatus::Failed:
      return L"failed";
    case SrfLookupCandidatesStatus::BackendNotConnected:
      return L"backend_not_connected";
  }
  return L"unknown";
}

bool LookupCandidatesStatusIsTransient(SrfLookupCandidatesStatus status) {
  switch (status) {
    case SrfLookupCandidatesStatus::EngineNotReady:
    case SrfLookupCandidatesStatus::BridgeBusy:
    case SrfLookupCandidatesStatus::EnsureFailed:
    case SrfLookupCandidatesStatus::Superseded:
    case SrfLookupCandidatesStatus::RemoteBusy:
    case SrfLookupCandidatesStatus::TransientFailure:
      return true;
    case SrfLookupCandidatesStatus::Ok:
    case SrfLookupCandidatesStatus::Empty:
    case SrfLookupCandidatesStatus::Failed:
    case SrfLookupCandidatesStatus::BackendNotConnected:
      return false;
  }
  return false;
}

bool CSrfTip::ApplyCandidateRefreshResult(const std::wstring& reading,
                                          std::vector<std::wstring> nextCandidates,
                                          std::vector<CandidateMetaParts> nextMeta,
                                          SrfEngineState stateAfterLookup,
                                          SrfLookupCandidatesStatus lookupStatus, bool asyncResult,
                                          unsigned long long requestId, bool hasMore,
                                          bool fullResult) {
  if (reading != m_compositionState.reading) {
    if (fullResult && CompleteCandidateFullLookupState(requestId)) {
      ResetCandidateFullLookupState();
    }
    if (!m_compositionState.reading.empty()) {
      if (!m_candidateView.candidates.empty() &&
          m_candidateView.candidatesReading != m_compositionState.reading) {
        SetCandidateViewState(SrfCandidateViewState::Stale, L"result-reading-mismatch");
      } else if (m_candidateView.candidates.empty()) {
        SetCandidateViewState(SrfCandidateViewState::Pending, L"result-reading-mismatch");
      }
    }
    std::wstring line = L"result_reading_units=" + std::to_wstring(reading.size());
    line += L", request_id=";
    line += std::to_wstring(requestId);
    line += L", current_reading_units=";
    line += std::to_wstring(m_compositionState.reading.size());
    SrfTsfPerfLog(asyncResult ? L"candidate-refresh.async-stale" : L"candidate-refresh.stale",
                  line.c_str());
    return false;
  }
  if (stateAfterLookup != SrfEngineState::Ready) {
    if (fullResult && CompleteCandidateFullLookupState(requestId)) {
      ResetCandidateFullLookupState();
    }
    EnsureEngineInputReady();
    return false;
  }
  bool partialResult = false;
  bool prefixPlaceholder = false;
  const bool transientLookup = LookupCandidatesStatusIsTransient(lookupStatus);
  for (const auto& parts : nextMeta) {
    if (parts.partialResult) partialResult = true;
    if (parts.prefixPlaceholder) prefixPlaceholder = true;
  }
  const bool hasCurrentCandidatesForReading =
      !m_candidateView.candidates.empty() && m_candidateView.candidatesReading == reading;
  bool currentPartialResult = false;
  bool currentPrefixPlaceholder = false;
  if (hasCurrentCandidatesForReading) {
    for (const auto& parts : m_candidateView.candidateMeta) {
      if (parts.partialResult) currentPartialResult = true;
      if (parts.prefixPlaceholder) currentPrefixPlaceholder = true;
    }
  }
  auto resolvedViewState = [&]() {
    if (m_candidateView.candidates.empty()) return SrfCandidateViewState::Empty;
    if (m_candidateView.candidatesReading != m_compositionState.reading)
      return SrfCandidateViewState::Stale;
    return CurrentCandidatesPartial() ? SrfCandidateViewState::PartialCurrent
                                      : SrfCandidateViewState::Stable;
  };
  const bool lookupEmptyForReading = nextCandidates.empty();
  bool usedFallbackCandidate = false;
  bool suppressedAlphabeticFallback = false;
  if (nextCandidates.empty() &&
      (stateAfterLookup == SrfEngineState::Loading || stateAfterLookup == SrfEngineState::Failed) &&
      !IsRawAlphabeticReadingForCandidateFallback(reading)) {
    nextCandidates.push_back(reading);
    nextMeta.emplace_back();
    nextMeta.back().noLearn = true;
    usedFallbackCandidate = true;
  }
  if (nextCandidates.empty() && !hasCurrentCandidatesForReading &&
      IsRawAlphabeticReadingForCandidateFallback(reading)) {
    suppressedAlphabeticFallback = true;
  }
  const bool interimCandidateResult =
      !nextCandidates.empty() && (partialResult || prefixPlaceholder);
  const bool interimHasNewTop =
      !nextCandidates.empty() &&
      (!hasCurrentCandidatesForReading || m_candidateView.candidates.empty() ||
       nextCandidates.front() != m_candidateView.candidates.front());
  const bool holdAsyncReadyPartial =
      interimCandidateResult && asyncResult && partialResult && !prefixPlaceholder &&
      stateAfterLookup == SrfEngineState::Ready && !m_candidateView.candidates.empty() &&
      !currentPrefixPlaceholder;
  const bool applyInterimCandidateResult =
      interimCandidateResult && !holdAsyncReadyPartial &&
      (!hasCurrentCandidatesForReading || currentPrefixPlaceholder ||
       (partialResult && !prefixPlaceholder && (currentPartialResult || interimHasNewTop)));
  const bool holdInterimCandidateResult =
      interimCandidateResult &&
      (holdAsyncReadyPartial || (hasCurrentCandidatesForReading && !applyInterimCandidateResult));
  if (holdInterimCandidateResult) {
    if (fullResult && CompleteCandidateFullLookupState(requestId)) {
      ResetCandidateFullLookupState();
    }
    SetCandidateViewState(resolvedViewState(), L"retain-interim");
    ClampCandidateState();
    MaybeNotifyEngineHealth();
    RebuildContextModel();
    SyncStatusModel();
    std::wstring line = L"request_id=" + std::to_wstring(requestId);
    line += L", reading=";
    line += ShortenForLog(m_compositionState.reading);
    line += L", count=";
    line += std::to_wstring(m_candidateView.candidates.size());
    line += L", next_count=";
    line += std::to_wstring(nextCandidates.size());
    line += L", retained_interim=1";
    if (holdAsyncReadyPartial) {
      line += L", reason=async_partial_ready";
    }
    if (asyncResult) {
      line += L", async=1";
    }
    if (m_candidateView.candidatesReading != m_compositionState.reading) {
      line += L", retained_stale=1";
    }
    if (partialResult) {
      line += L", partial=1";
    }
    if (prefixPlaceholder) {
      line += L", prefix_placeholder=1";
    }
    SrfTsfPerfLog(L"candidate-refresh.end", line.c_str());

    const bool awaitingQueuedAsyncLookup = prefixPlaceholder && !asyncResult;
    if (awaitingQueuedAsyncLookup) {
      SrfTsfPerfLog(L"candidate-refresh.interim", L"retry=await_async");
    } else if (partialResult) {
      ScheduleDeferredCandidateRefresh(kDeferredCandidateIdleRetryMs);
      SrfTsfPerfLog(L"candidate-refresh.interim", L"retry=idle_scheduled");
    } else {
      ScheduleDeferredCandidateRefresh();
      SrfTsfPerfLog(L"candidate-refresh.interim", L"retry=scheduled");
    }
    return false;
  }
  const bool currentPartialHasSelectablePhrase =
      m_candidateView.candidates.size() > 1 ||
      (!m_candidateView.candidates.empty() && m_candidateView.candidates.front() != reading);
  const bool completingInteractiveSnapshot =
      hasCurrentCandidatesForReading && !currentPrefixPlaceholder && !partialResult &&
      !prefixPlaceholder && !nextCandidates.empty() && currentPartialHasSelectablePhrase &&
      (asyncResult || currentPartialResult);
  const bool trackedCurrentPartialSnapshot =
      currentPartialResult && m_candidateView.partialCandidateInteractionReading == reading;
  const bool canRefreshCompletedPartialSnapshot =
      srf_candidate_stability::CanRefreshCompletedPartialSnapshot(
          currentPartialResult, trackedCurrentPartialSnapshot,
          m_candidateView.candidateInteractionVersion,
          m_candidateView.partialCandidateInteractionVersion);
  bool frozeInteractiveSnapshot = false;
  if (completingInteractiveSnapshot && canRefreshCompletedPartialSnapshot) {
    srf_candidate_stability::StabilizeCompletedBatch(m_candidateView.candidates,
                                                     m_candidateView.candidateMeta, &nextCandidates,
                                                     &nextMeta, srf_candidate_limits::kFullResult);
  }
  if (completingInteractiveSnapshot && !canRefreshCompletedPartialSnapshot) {
    // Preserve exact indexes after interaction. Untouched partial batches use
    // the filtered stable merge above instead of replacing their visible order.
    frozeInteractiveSnapshot = srf_candidate_stability::FreezeInteractiveBatch(
        m_candidateView.candidates, m_candidateView.candidateMeta, &nextCandidates, &nextMeta,
        srf_candidate_limits::kFullResult);
  }
  const bool retainEmptyTransientResult = srf_candidate_stability::ShouldRetainEmptyCandidateResult(
      !m_compositionState.reading.empty(), nextCandidates.empty(),
      m_candidateView.candidates.empty(), transientLookup);
  const bool retainedPrevious = retainEmptyTransientResult;
  const bool retainedStaleCandidates = retainedPrevious && !hasCurrentCandidatesForReading;
  const bool unchangedCandidates =
      !retainedPrevious && m_candidateView.candidatesReading == m_compositionState.reading &&
      nextCandidates == m_candidateView.candidates && nextMeta == m_candidateView.candidateMeta;
  if (!retainedPrevious && !unchangedCandidates) {
    m_candidateView.candidates = std::move(nextCandidates);
    m_candidateView.candidateMeta = std::move(nextMeta);
    m_candidateView.candidatesReading =
        m_candidateView.candidates.empty() ? std::wstring() : m_compositionState.reading;
    RebuildCandidateRows();
    InvalidateCandidatePageLayoutCache();
    // Prefetch long clipboard entries (clipboard://{id} placeholders) so the
    // commit path resolves from the local cache instead of a blocking pipe
    // wait on the key thread.  Idempotent; cached ids are skipped.
    {
      std::vector<std::wstring> prefetchIds;
      const size_t metaCount =
          std::min(m_candidateView.candidates.size(), m_candidateView.candidateMeta.size());
      for (size_t i = 0; i < metaCount; ++i) {
        if (m_candidateView.candidates[i].rfind(L"clipboard://", 0) != 0) continue;
        const CandidateMetaParts& parts = m_candidateView.candidateRows[i].meta;
        if (parts.clipboardQuick && !parts.clipboardId.empty()) {
          prefetchIds.push_back(parts.clipboardId);
        }
      }
      SrfTip_PrefetchClipboardTexts(prefetchIds);
    }
  }
  if (!retainedPrevious &&
      (lookupStatus == SrfLookupCandidatesStatus::Ok || !m_candidateView.candidates.empty())) {
    m_candidateView.candidateHasMore = hasMore;
  }
  const UINT priorCandidatePage = m_candidateView.candPage;
  const UINT priorCandidateSelection = m_candidateView.candSel;
  if (fullResult && CompleteCandidateFullLookupState(requestId)) {
    if (!retainedPrevious && lookupStatus == SrfLookupCandidatesStatus::Ok) {
      if (m_candidateView.candidateIndexAfterLoad != 0 &&
          m_candidateView.candidateIndexAfterLoad < m_candidateView.candidates.size()) {
        m_candidateView.candSel = m_candidateView.candidateIndexAfterLoad;
        m_candidateView.candPage = CandidatePageForIndex(m_candidateView.candSel);
      } else if (m_candidateView.candidatePageAfterLoad != 0) {
        m_candidateView.candPage =
            std::min(m_candidateView.candidatePageAfterLoad, MaxCandidatePage());
        const UINT pageStart = CandidatePageStart(m_candidateView.candPage);
        const UINT pageEndExclusive = CandidatePageEndExclusive(m_candidateView.candPage);
        if (m_candidateView.candSel < pageStart || m_candidateView.candSel >= pageEndExclusive) {
          m_candidateView.candSel = pageStart;
        }
      }
    }
    ResetCandidateFullLookupState();
  }
  SetCandidateViewState(resolvedViewState(),
                        retainedPrevious ? L"retain-transient"
                                         : (unchangedCandidates ? L"unchanged" : L"apply-result"));
  if (!retainedPrevious && !unchangedCandidates && lookupStatus == SrfLookupCandidatesStatus::Ok &&
      !usedFallbackCandidate && !prefixPlaceholder && !m_candidateView.candidates.empty()) {
    RememberPrefixCandidateCache(m_compositionState.reading);
  }
  ClampCandidateState();
  MaybeNotifyEngineHealth();
  RebuildContextModel();
  SyncStatusModel();
  if (!retainedPrevious && lookupStatus == SrfLookupCandidatesStatus::Ok &&
      m_candidateView.clipboardQuickLastPageReading == reading &&
      CurrentCandidatesClipboardQuickMode()) {
    // Crossing back into an earlier eight-entry batch lands on its last
    // visible page, so consecutive previous-page presses never skip rows.
    m_candidateView.candPage = MaxCandidatePage();
    m_candidateView.candSel = CandidatePageStart(m_candidateView.candPage);
    m_candidateView.clipboardQuickLastPageReading.clear();
    ClampCandidateState();
    SyncCandidateContextState();
  }
  if (!retainedPrevious && !unchangedCandidates && partialResult && !prefixPlaceholder &&
      !m_candidateView.candidates.empty()) {
    if (m_candidateView.candSel == 0 && m_candidateView.candPage == 0) {
      m_candidateView.partialCandidateInteractionReading = m_compositionState.reading;
      m_candidateView.partialCandidateInteractionVersion =
          m_candidateView.candidateInteractionVersion;
    } else {
      m_candidateView.partialCandidateInteractionReading.clear();
    }
  } else if (!retainedPrevious && !unchangedCandidates && !partialResult) {
    m_candidateView.partialCandidateInteractionReading.clear();
    m_candidateView.partialCandidateInteractionVersion =
        m_candidateView.candidateInteractionVersion;
  }
  if (!m_compositionState.reading.empty()) {
    std::wstring endLine = L"request_id=" + std::to_wstring(requestId);
    endLine += L", reading=";
    endLine += ShortenForLog(m_compositionState.reading);
    endLine += L", count=";
    endLine += std::to_wstring(m_candidateView.candidates.size());
    endLine += L", engine=";
    endLine += EngineStateName(stateAfterLookup);
    if (asyncResult) {
      endLine += L", async=1";
    }
    if (retainedPrevious) {
      endLine += L", retained=1";
    }
    if (retainedStaleCandidates) {
      endLine += L", retained_stale=1";
    }
    if (usedFallbackCandidate) {
      endLine += L", fallback=1";
    }
    if (suppressedAlphabeticFallback) {
      endLine += L", raw_fallback_suppressed=1";
    }
    if (partialResult) {
      endLine += L", partial=1";
    }
    if (lookupStatus != SrfLookupCandidatesStatus::Ok) {
      endLine += L", lookup_status=";
      endLine += LookupCandidatesStatusName(lookupStatus);
    }
    if (prefixPlaceholder) {
      endLine += L", prefix_placeholder=1";
    }
    if (unchangedCandidates) {
      endLine += L", status=unchanged";
    }
    if (frozeInteractiveSnapshot) {
      endLine += L", froze_interactive_snapshot=1";
    }
    if (canRefreshCompletedPartialSnapshot && !unchangedCandidates) {
      endLine += L", refreshed_completed_partial=1";
    }
    endLine += L", view_state=";
    endLine += CandidateViewStateName();
    if (!m_candidateView.candidates.empty()) {
      endLine += L", top=";
      endLine += ShortenForLog(m_candidateView.candidates.front(), 24);
    }
    SrfTsfPerfLog(L"candidate-refresh.end", endLine.c_str());
  }
  if (!m_compositionState.reading.empty() &&
      (lookupEmptyForReading || m_candidateView.candidates.empty() || retainedPrevious ||
       usedFallbackCandidate || partialResult || transientLookup || hasMore)) {
    const SrfEngineState state = stateAfterLookup;
    std::wstring detail = L"request_id=" + std::to_wstring(requestId);
    detail += L", reading=";
    detail += ShortenForLog(m_compositionState.reading);
    detail += L", engine=";
    detail += EngineStateName(state);
    detail += L", lookup_status=";
    detail += LookupCandidatesStatusName(lookupStatus);
    if (retainedPrevious) detail += L", retained=1";
    if (retainedStaleCandidates) detail += L", retained_stale=1";
    if (usedFallbackCandidate) detail += L", fallback=1";
    if (suppressedAlphabeticFallback) detail += L", raw_fallback_suppressed=1";
    if (partialResult) detail += L", partial=1";
    const std::wstring failure = SrfTip_GetEngineFailureDetail();
    if (!failure.empty()) {
      detail += L", failure=";
      detail += ShortenForLog(failure, 96);
    }
    const size_t visibleCandidateTarget =
        m_uiStyle.candidateHorizontal
            ? static_cast<size_t>(std::clamp(m_uiStyle.candidateHorizontalCount, 3u, 9u))
            : static_cast<size_t>(CandidatePageSize());
    const bool hasCurrentUsableCandidates =
        !m_candidateView.candidates.empty() &&
        m_candidateView.candidatesReading == m_compositionState.reading && !usedFallbackCandidate &&
        !prefixPlaceholder;
    // A non-full lookup may return a short first chunk and advertise that more
    // candidates are available. Request the complete result now, rather than
    // making the user press PageDown to trigger the same lookup path.
    const bool incompleteFirstPage = lookupStatus == SrfLookupCandidatesStatus::Ok && !fullResult &&
                                     hasMore && !IsCandidateFullLookupPending() &&
                                     hasCurrentUsableCandidates &&
                                     m_candidateView.candidates.size() < visibleCandidateTarget;
    const bool hasFirstPaintCandidates =
        hasCurrentUsableCandidates &&
        (!partialResult || m_candidateView.candidates.size() >= visibleCandidateTarget);
    const bool retryIdle = hasCurrentUsableCandidates &&
                           (transientLookup || (partialResult && hasFirstPaintCandidates));
    const bool retryNow = (transientLookup && !hasCurrentUsableCandidates) ||
                          (partialResult && !hasFirstPaintCandidates) ||
                          state == SrfEngineState::Loading ||
                          (state == SrfEngineState::Failed && m_config.engine.retryOnFailure);
    const bool awaitingQueuedAsyncLookup = prefixPlaceholder && !asyncResult;
    if (awaitingQueuedAsyncLookup) {
      detail += L", retry=await_async";
    } else if (incompleteFirstPage) {
      detail += L", retry=full_result";
      RefreshCandidatesAsync(true);
    } else if (retryNow) {
      detail += L", retry=scheduled";
      ScheduleDeferredCandidateRefresh();
    } else if (retryIdle) {
      detail += L", retry=idle_scheduled";
      ScheduleDeferredCandidateRefresh(kDeferredCandidateIdleRetryMs);
    } else {
      detail += L", retry=skipped";
    }
    SrfTsfPerfLog(L"candidate-refresh.empty", detail.c_str());
  }
  return !retainedPrevious &&
         (!unchangedCandidates || m_candidateView.candPage != priorCandidatePage ||
          m_candidateView.candSel != priorCandidateSelection);
}

void CSrfTip::ApplyAsyncCandidateResult(TfEditCookie ec) {
  std::wstring reading;
  std::vector<std::wstring> candidates;
  std::vector<CandidateMetaParts> meta;
  SrfEngineState stateAfterLookup = SrfEngineState::Idle;
  SrfLookupCandidatesStatus lookupStatus = SrfLookupCandidatesStatus::Failed;
  bool hasMore = false;
  bool fullResult = false;
  unsigned long long serial = 0;
  ULONGLONG requestTick = 0;
  SrfFocusSnapshot focus = {};
  {
    std::lock_guard<std::mutex> guard(m_asyncCandidateMutex);
    if (!m_asyncCandidatePending) return;
    m_asyncCandidatePending = false;
    serial = m_asyncCandidatePendingSerial;
    reading = std::move(m_asyncCandidatePendingReading);
    candidates = std::move(m_asyncCandidatePendingItems);
    meta = std::move(m_asyncCandidatePendingMeta);
    stateAfterLookup = m_asyncCandidatePendingEngineState;
    requestTick = m_asyncCandidatePendingRequestTick;
    focus = m_asyncCandidatePendingFocus;
    m_asyncCandidatePendingReading.clear();
    m_asyncCandidatePendingItems.clear();
    m_asyncCandidatePendingMeta.clear();
    lookupStatus = m_asyncCandidatePendingLookupStatus;
    hasMore = m_asyncCandidatePendingHasMore;
    fullResult = m_asyncCandidatePendingFullResult;
    m_asyncCandidatePendingLookupStatus = SrfLookupCandidatesStatus::Ok;
    m_asyncCandidatePendingHasMore = false;
    m_asyncCandidatePendingFullResult = false;
    m_asyncCandidatePendingRequestTick = 0;
    m_asyncCandidatePendingFocus = {};
  }
  if (ShouldSuppressCandidatesForPrivacy()) {
    m_candidateView.candidates.clear();
    m_candidateView.candidateMeta.clear();
    m_candidateView.candidateRows.clear();
    m_candidateView.candidatesReading.clear();
    m_candidateView.candidateHasMore = false;
    ResetCandidateFullLookupState();
    SetCandidateViewState(SrfCandidateViewState::Empty, L"async-drop-privacy");
    InvalidateCandidatePageLayoutCache();
    ClampCandidateState();
    RebuildContextModel();
    SyncStatusModel();
    RedrawCandidateUi();
    SrfTsfPerfLog(L"candidate-refresh.async-drop", L"reason=privacy_disabled");
    return;
  }

  const unsigned long long currentSerial = m_candidateLookupSerial.load(std::memory_order_acquire);
  if (serial != currentSerial || reading != m_compositionState.reading ||
      m_compositionState.reading.empty()) {
    if (fullResult && CompleteCandidateFullLookupState(serial)) {
      ResetCandidateFullLookupState();
    }
    if (!m_compositionState.reading.empty()) {
      if (!m_candidateView.candidates.empty() &&
          m_candidateView.candidatesReading != m_compositionState.reading) {
        SetCandidateViewState(SrfCandidateViewState::Stale, L"async-drop-stale");
      } else if (m_candidateView.candidates.empty()) {
        SetCandidateViewState(SrfCandidateViewState::Pending, L"async-drop-pending");
      }
    }
    std::wstring line = L"reading_units=" + std::to_wstring(reading.size());
    line += L", request_id=";
    line += std::to_wstring(serial);
    line += L", currentSerial=";
    line += std::to_wstring(currentSerial);
    line += L", current_reading_units=";
    line += std::to_wstring(m_compositionState.reading.size());
    if (serial != currentSerial) {
      line += L", reason=serial";
    } else if (reading != m_compositionState.reading) {
      line += L", reason=reading";
    } else {
      line += L", reason=empty_current";
    }
    SrfTsfPerfLog(L"candidate-refresh.async-drop", line.c_str());
    return;
  }

  if (!FocusSnapshotMatches(focus)) {
    if (fullResult && CompleteCandidateFullLookupState(serial)) {
      ResetCandidateFullLookupState();
    }
    if (!m_candidateView.candidates.empty() &&
        m_candidateView.candidatesReading != m_compositionState.reading) {
      SetCandidateViewState(SrfCandidateViewState::Stale, L"async-drop-focus");
    } else if (m_candidateView.candidates.empty() && !m_compositionState.reading.empty()) {
      SetCandidateViewState(SrfCandidateViewState::Pending, L"async-drop-focus");
    }
    std::wstring line = L"request_id=" + std::to_wstring(serial);
    line += L", reading_units=";
    line += std::to_wstring(reading.size());
    line += L", requested=";
    line += FormatFocusSnapshotForLog(focus);
    line += L", current=";
    line += FormatFocusSnapshotForLog(CaptureFocusSnapshot(
        m_compositionState.pCompositionContext ? m_compositionState.pCompositionContext
                                               : m_pFocusContext));
    SrfTsfPerfLog(L"candidate-refresh.async-drop-focus", line.c_str());
    return;
  }

  if (requestTick != 0) {
    DebugLogPerfMs(L"CandidateWorker/request-to-apply", requestTick);
  }
  const bool changed =
      ApplyCandidateRefreshResult(reading, std::move(candidates), std::move(meta), stateAfterLookup,
                                  lookupStatus, true, serial, hasMore, fullResult);
  // A result that is byte-for-byte identical to the current stable snapshot does not
  // need to walk the TSF UI-element/update/measure path again.  The synchronous
  // callers still explicitly update the window when they need to show it for the
  // first time; this branch is only for an already delivered async result.
  if (changed) {
    UpdateCandidateWindow(ec);
    if (requestTick != 0) {
      DebugLogPerfMs(fullResult ? L"CandidateWorker/full-request-to-window"
                                : L"CandidateWorker/first-request-to-window",
                     requestTick);
    }
  }
}

UINT CSrfTip::CandidatePageSize() const {
  return std::clamp(m_uiStyle.candidatePageSize, 3u, 10u);
}

std::wstring CSrfTip::CandidateBarMainTitle() const {
  if (CurrentCandidatesClipboardQuickMode()) return L"\u5feb\u6377\u526a\u8d34\u677f";
  return BuildCompositionDisplay();
}

std::vector<std::wstring> CSrfTip::CandidateBarModeTags() const {
  if (m_gameChat.chatActive && EffectiveGameStatusIndicator()) return {L"中文聊天"};
  if (CurrentCandidatesClipboardQuickMode()) {
    for (const auto& row : m_candidateView.candidateRows) {
      if (!row.meta.clipboardFilter.empty()) return {row.meta.clipboardFilter};
    }
    return {};
  }
  if (!m_uiStyle.showModeInCandidateHeader) {
    m_lastCandidateModeTags.clear();
    m_candidateModeTagsVisibleUntilTick = 0;
    return {};
  }
  std::vector<std::wstring> tags;
  tags.reserve(5);
  tags.push_back(m_imeOpen ? L"\u4e2d" : L"\u82f1");
  tags.push_back(m_fullShape ? L"\u5168\u89d2" : L"\u534a\u89d2");
  tags.push_back(m_cnPunct ? L"\u4e2d\u6807" : L"\u82f1\u6807");
  tags.push_back(m_doublePinyin ? L"\u53cc\u62fc" : L"\u5168\u62fc");
  if (m_fuzzyPinyin) tags.push_back(L"\u6a21\u7cca");
  const ULONGLONG now = GetTickCount64();
  if (tags != m_lastCandidateModeTags) {
    m_lastCandidateModeTags = tags;
    m_candidateModeTagsVisibleUntilTick = now + 1600;
  }
  if (now <= m_candidateModeTagsVisibleUntilTick) {
    const ULONGLONG remaining = m_candidateModeTagsVisibleUntilTick - now + 1;
    const_cast<CSrfTip*>(this)->ScheduleCandidateUiRedraw(
        static_cast<DWORD>(std::min<ULONGLONG>(remaining, MAXDWORD)));
    return tags;
  }
  return {};
}

std::wstring CSrfTip::FormatCandidateDisplayText(size_t index) const {
  if (index >= m_candidateView.candidates.size()) return {};

  const CandidateMetaParts* meta = index < m_candidateView.candidateRows.size()
                                       ? &m_candidateView.candidateRows[index].meta
                                       : nullptr;
  std::wstring text =
      !meta || meta->display.empty() ? m_candidateView.candidates[index] : meta->display;
  SrfUIStyle quietStyle = EffectiveCandidateUiStyle();
  quietStyle.showCandidateSource = false;
  if (meta) PrefixCandidateDisplayText(&text, *meta, quietStyle);
  return text;
}

const std::vector<std::wstring>& CSrfTip::BuildCandidateDisplayItems() const {
  if (m_candidateDisplayItemsCacheValid) return m_candidateDisplayItemsCache;
  m_candidateDisplayItemsCache.clear();
  m_candidateDisplayItemsCache.reserve(m_candidateView.candidates.size());
  const SrfUIStyle style = EffectiveCandidateUiStyle();
  for (size_t i = 0; i < m_candidateView.candidates.size(); ++i) {
    const CandidateMetaParts* meta =
        i < m_candidateView.candidateRows.size() ? &m_candidateView.candidateRows[i].meta : nullptr;
    std::wstring text =
        !meta || meta->display.empty() ? m_candidateView.candidates[i] : meta->display;
    // 来源图标仅跟随当前候选显示；纠错提示仍需覆盖全部候选，避免失去语义。
    SrfUIStyle quietStyle = style;
    quietStyle.showCandidateSource = false;
    if (meta) PrefixCandidateDisplayText(&text, *meta, quietStyle);
    m_candidateDisplayItemsCache.push_back(std::move(text));
  }
  m_candidateDisplayItemsCacheValid = true;
  return m_candidateDisplayItemsCache;
}

bool CSrfTip::CurrentCandidatesForceVerticalLayout() const {
  for (const auto& row : m_candidateView.candidateRows) {
    if (row.meta.forceVerticalLayout) return true;
  }
  return false;
}

bool CSrfTip::CurrentCandidatesClipboardQuickMode() const {
  UINT page = 0;
  std::wstring filter;
  if (ParseClipboardQuickReading(m_compositionState.reading, &page, &filter)) return true;
  for (const auto& row : m_candidateView.candidateRows) {
    if (row.meta.clipboardQuick) return true;
  }
  return false;
}

bool CSrfTip::IsClipboardQuickCandidate(size_t idx) const {
  return idx < m_candidateView.candidateRows.size() &&
         m_candidateView.candidateRows[idx].meta.clipboardQuick;
}

bool CSrfTip::IsClipboardCandidatePinned(size_t idx) const {
  return idx < m_candidateView.candidateRows.size() &&
         m_candidateView.candidateRows[idx].meta.clipboardPinned;
}

bool CSrfTip::ClipboardCandidatePageInfo(size_t idx, UINT* page, UINT* pages) const {
  if (!page || !pages || idx >= m_candidateView.candidateRows.size()) {
    return false;
  }
  const CandidateMetaParts& meta = m_candidateView.candidateRows[idx].meta;
  if (!meta.clipboardQuick) return false;
  *page = meta.clipboardPage;
  *pages = std::max(1u, meta.clipboardPages);
  return true;
}

SrfUIStyle CSrfTip::EffectiveCandidateUiStyle() const {
  SrfUIStyle style = m_uiStyle;
  style.candidateFullscreenPlacement = FullscreenCandidateOverlayActive();
  if ((m_fullscreenCompatActive || IsGameHotkeyPassthroughActive()) &&
      EffectiveCompatibilityPolicy() == SrfFullscreenPolicy::ShowUi) {
    style.candidateHorizontal = true;
    style.candidateHorizontalCompact = true;
    style.candidatePageSize = std::min(style.candidatePageSize, 5u);
    style.candidateFieldWidthDip = m_gameChat.candidateFieldWidthDip;
    style.candidateHorizontalCount =
        style.candidateFieldWidthDip ? 9u : std::min(style.candidateHorizontalCount, 5u);
    style.candidateTopmost = true;
    style.candidateOpacity = 100;
    style.candidateMaterial = SrfCandidateMaterial::Solid;
    style.candidateDensity = SrfCandidateDensity::Compact;
    style.candidateLayoutVariant = SrfCandidateLayoutVariant::Compact;
    style.skinAnimationsEnabled = false;
    if (style.candidateOverlayAnchor == SrfOverlayAnchor::Auto)
      style.candidateOverlayAnchor = SrfOverlayAnchor::BottomLeft;
    style.candidateLeftClick = false;
    style.candidateRightClick = false;
    style.showCandidateReading = false;
    style.showCandidateScore = false;
    style.showCandidateSource = false;
    style.showModeInCandidateHeader = false;
  }
  if (CurrentCandidatesForceVerticalLayout()) {
    style.candidateHorizontal = false;
    style.candidateHorizontalCompact = false;
  }
  if (CurrentCandidatesClipboardQuickMode()) {
    style.candidateHorizontal = false;
    style.candidateHorizontalCompact = false;
    style.candidateLayoutVariant = SrfCandidateLayoutVariant::Card;
    style.candidateDensity = SrfCandidateDensity::Comfortable;
    style.candidateRightClick = false;

    const POINT anchor = {m_anchor.lastCandidateRect.left, m_anchor.lastCandidateRect.top};
    if (!m_clipboardPageSize || m_clipboardPageUsesAnchor != m_anchor.hasLastCandidateRect ||
        (m_anchor.hasLastCandidateRect && !PtInRect(&m_clipboardPageMonitorRect, anchor))) {
      RECT work = {0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)};
      m_clipboardPageMonitorRect = work;
      const RECT* anchorRect =
          m_anchor.hasLastCandidateRect ? &m_anchor.lastCandidateRect : nullptr;
      if (anchorRect) {
        const HMONITOR monitor = MonitorFromRect(anchorRect, MONITOR_DEFAULTTONEAREST);
        MONITORINFO info = {sizeof(info)};
        if (monitor && GetMonitorInfoW(monitor, &info)) {
          work = info.rcWork;
          m_clipboardPageMonitorRect = info.rcMonitor;
        }
      }
      m_clipboardPageUsesAnchor = m_anchor.hasLastCandidateRect;
      const int dpi = static_cast<int>(DpiForScreenRect(anchorRect));
      const int height = MulDiv(std::max<LONG>(1, work.bottom - work.top), 96, dpi);
      m_clipboardPageSize = height >= 1000 ? 8u : height >= 850 ? 7u : height >= 720 ? 6u : 5u;
    }
    style.candidatePageSize = m_clipboardPageSize;
  }
  return style;
}

CandidatePageLayoutMetrics CSrfTip::BuildCandidatePageLayout() const {
  if (m_candidatePageLayoutCacheValid) return m_candidatePageLayoutCache;
  const auto& items = BuildCandidateDisplayItems();
  m_candidatePageLayoutCache = BuildCandidatePageLayoutMetrics(
      EffectiveCandidateUiStyle(),
      m_anchor.hasLastCandidateRect ? &m_anchor.lastCandidateRect : nullptr, items);
  m_candidatePageLayoutCacheValid = true;
  return m_candidatePageLayoutCache;
}

void CSrfTip::InvalidateCandidatePageLayoutCache() {
  ++m_candidateDisplayVersion;
  m_candidatePageLayoutCacheValid = false;
  m_candidatePageLayoutCache = {};
  m_candidateDisplayItemsCacheValid = false;
  m_candidateDisplayItemsCache.clear();
}

UINT CSrfTip::CandidatePageCount() const {
  if (m_candidateView.candidates.empty()) return 1;
  const auto layout = BuildCandidatePageLayout();
  return std::max(1u, static_cast<UINT>(layout.pageStarts.size()));
}

UINT CSrfTip::CandidatePageStart(UINT page) const {
  if (m_candidateView.candidates.empty()) return 0;
  const auto layout = BuildCandidatePageLayout();
  return PageStartFromMetrics(layout, page);
}

UINT CSrfTip::CandidatePageEndExclusive(UINT page) const {
  if (m_candidateView.candidates.empty()) return 0;
  const auto layout = BuildCandidatePageLayout();
  return PageEndExclusiveFromMetrics(layout, page,
                                     static_cast<UINT>(m_candidateView.candidates.size()));
}

UINT CSrfTip::CandidatePageForIndex(UINT index) const {
  if (m_candidateView.candidates.empty()) return 0;
  const auto layout = BuildCandidatePageLayout();
  return PageForIndexFromMetrics(layout, index,
                                 static_cast<UINT>(m_candidateView.candidates.size()));
}

UINT CSrfTip::CandidateIndexInPage(UINT index) const {
  if (m_candidateView.candidates.empty()) return 0;
  const UINT page = CandidatePageForIndex(index);
  return index - CandidatePageStart(page);
}

void CSrfTip::ClampReadingCursor() {
  m_compositionState.ClampCursor();
}

UINT CSrfTip::MaxCandidatePage() const {
  if (m_candidateView.candidates.empty()) return 0;
  return CandidatePageCount() - 1;
}

bool CSrfTip::IsCandidateFullLookupPending() {
  if (!m_candidateView.candidateFullLookupPending) return false;
  const unsigned long long currentSerial = m_candidateLookupSerial.load(std::memory_order_acquire);
  if (m_candidateView.candidateFullLookupSerial == 0 ||
      m_candidateView.candidateFullLookupSerial != currentSerial) {
    ResetCandidateFullLookupState();
    return false;
  }
  return true;
}

void CSrfTip::ResetCandidateFullLookupState() {
  m_candidateView.ResetFullLookup();
}

bool CSrfTip::CompleteCandidateFullLookupState(unsigned long long requestId) {
  return m_candidateView.CompleteFullLookup(requestId);
}

bool CSrfTip::RequestMoreCandidatesForPage(UINT targetPage) {
  if (!m_candidateView.candidateHasMore || m_compositionState.reading.empty()) return false;
  if (targetPage <= MaxCandidatePage()) return false;
  const bool pending = IsCandidateFullLookupPending();
  ++m_candidateView.candidateInteractionVersion;
  m_candidateView.candidateIndexAfterLoad = 0;
  m_candidateView.candidatePageAfterLoad =
      std::max(m_candidateView.candidatePageAfterLoad, targetPage);
  if (!pending) RefreshCandidatesAsync(true);
  return true;
}

bool CSrfTip::RequestMoreCandidatesForIndex(UINT targetIndex) {
  if (!m_candidateView.candidateHasMore || m_compositionState.reading.empty() ||
      targetIndex < m_candidateView.candidates.size())
    return false;
  const bool pending = IsCandidateFullLookupPending();
  ++m_candidateView.candidateInteractionVersion;
  m_candidateView.candidatePageAfterLoad = 0;
  m_candidateView.candidateIndexAfterLoad = targetIndex;
  if (!pending) RefreshCandidatesAsync(true);
  return true;
}

bool CSrfTip::RequestMoreCandidatesToFillCurrentPage() {
  if (!m_candidateView.candidateHasMore || IsCandidateFullLookupPending() ||
      m_compositionState.reading.empty() ||
      m_candidateView.candidatesReading != m_compositionState.reading ||
      m_candidateView.candPage != MaxCandidatePage()) {
    return false;
  }

  const UINT pageStart = CandidatePageStart(m_candidateView.candPage);
  const UINT pageEndExclusive = CandidatePageEndExclusive(m_candidateView.candPage);
  const size_t visibleCandidateTarget =
      m_uiStyle.candidateHorizontal
          ? static_cast<size_t>(std::clamp(m_uiStyle.candidateHorizontalCount, 3u, 9u))
          : static_cast<size_t>(CandidatePageSize());
  if (pageEndExclusive <= pageStart ||
      static_cast<size_t>(pageEndExclusive - pageStart) >= visibleCandidateTarget) {
    return false;
  }

  // The first lookup is intentionally capped. When that cap leaves the last
  // visible page short, complete it in the background without requiring one
  // more PageDown solely to request the remaining candidates.
  RefreshCandidatesAsync(true);
  return true;
}

void CSrfTip::ClampCandidateState() {
  if (m_candidateView.candidates.empty()) {
    m_candidateView.candSel = 0;
    m_candidateView.candPage = 0;
    return;
  }

  if (m_candidateView.candSel >= m_candidateView.candidates.size())
    m_candidateView.candSel = static_cast<UINT>(m_candidateView.candidates.size() - 1);
  if (m_candidateView.candPage > MaxCandidatePage()) m_candidateView.candPage = MaxCandidatePage();

  const auto layout = BuildCandidatePageLayout();
  const UINT pageStart = PageStartFromMetrics(layout, m_candidateView.candPage);
  const UINT pageEndExclusive = PageEndExclusiveFromMetrics(
      layout, m_candidateView.candPage, static_cast<UINT>(m_candidateView.candidates.size()));
  // 仅当高亮已离开当前页时，才用选择推导页码；否则显式翻页（PgUp/PgDn、SetPageIndex）会被错误打回。
  if (m_candidateView.candSel < pageStart || m_candidateView.candSel >= pageEndExclusive) {
    m_candidateView.candPage = PageForIndexFromMetrics(
        layout, m_candidateView.candSel, static_cast<UINT>(m_candidateView.candidates.size()));
  }
  RequestMoreCandidatesToFillCurrentPage();
}
