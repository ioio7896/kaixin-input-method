#include "ime_model.h"

#include <initializer_list>

int main() {
  using Backend = SrfOverlayBackend;

  if (ShouldUseExternalCandidateOverlayBackend(Backend::Auto, false, false,
                                                false)) {
    return 1;
  }
  if (!ShouldUseExternalCandidateOverlayBackend(Backend::Auto, true, false,
                                                 false)) {
    return 2;
  }
  if (!ShouldUseExternalCandidateOverlayBackend(Backend::Auto, false, true,
                                                 false)) {
    return 3;
  }
  if (!ShouldUseExternalCandidateOverlayBackend(Backend::External, false,
                                                 false, false)) {
    return 4;
  }
  if (ShouldUseExternalCandidateOverlayBackend(Backend::InProcess, true, true,
                                                false)) {
    return 5;
  }
  for (const Backend backend : {Backend::Auto, Backend::InProcess,
                                Backend::External}) {
    if (ShouldUseExternalCandidateOverlayBackend(backend, true, true, true)) {
      return 6;
    }
  }
  return 0;
}
