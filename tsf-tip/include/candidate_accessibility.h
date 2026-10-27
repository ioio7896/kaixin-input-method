#pragma once
#include <windows.h>
#include <memory>
#include <string>
#include <vector>

struct SrfCandidateAccessibleState;
struct IRawElementProviderSimple;

// Providers retain a thread-safe snapshot, never a pointer to the TIP or window.
// UIA clients may keep a provider alive after composition/window destruction.
class SrfCandidateAccessibility {
 public:
  static constexpr UINT kSelectMessage = WM_APP + 0x318;
  static constexpr UINT kInvokeMessage = WM_APP + 0x319;
  SrfCandidateAccessibility();
  ~SrfCandidateAccessibility();
  LRESULT GetObject(HWND hwnd, WPARAM wParam, LPARAM lParam);
  HRESULT ProviderForWindow(HWND hwnd, IRawElementProviderSimple** result);
  void Update(HWND hwnd, const std::wstring& title, const std::vector<std::wstring>& items,
              const std::vector<std::wstring>& comments, const std::vector<RECT>& itemRects,
              UINT page, UINT selection, bool interactive);
  void Hide();
  void Destroy();
  bool CanAct(UINT index, ULONG generation) const;

 private:
  std::shared_ptr<SrfCandidateAccessibleState> m_state;
  IRawElementProviderSimple* m_root = nullptr;
};
