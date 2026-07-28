#pragma once

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <string>

enum class SrfNotificationTone {
  Default,
  English,
  Chinese,
};

class CNotificationWindow {
 public:
  CNotificationWindow() = default;
  ~CNotificationWindow();

  void Show(const std::wstring& text, const RECT* anchorRect, UINT timeoutMs,
            SrfNotificationTone tone = SrfNotificationTone::Default);
  void Hide();
  void Destroy();

 private:
  HWND m_hwnd = nullptr;
  std::wstring m_text;
  SrfNotificationTone m_tone = SrfNotificationTone::Default;
  UINT m_dpi = 96;
  HFONT m_font = nullptr;

  bool EnsureWindow();
  void RefreshFont();
  void DestroyFont();
  RECT CalculateWindowRect(const RECT* anchorRect) const;
  SIZE MeasureWindow() const;
  int Scale(int value) const;
  int StrokeWidth() const;
  void Paint(HDC hdc);

  static ATOM EnsureWindowClass();
  static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam);
};
