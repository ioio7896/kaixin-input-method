#pragma once

#include <string>
#include "ime_model.h"

inline std::wstring SrfConvertDirectText(const std::wstring& text, const SrfInputOptions& input,
                                         bool chinesePunct, bool fullShape,
                                         bool& nextDoubleQuoteOpen, bool& nextSingleQuoteOpen) {
  std::wstring converted;
  converted.reserve(text.size() * 2);

  for (wchar_t ch : text) {
    if (input.chineseHalfwidth && ch >= 0x20 && ch <= 0x7e) {
      converted.push_back(ch);
      continue;
    }
    std::wstring replacement;
    if (chinesePunct) {
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
          replacement = nextDoubleQuoteOpen ? L"“" : L"”";
          nextDoubleQuoteOpen = !nextDoubleQuoteOpen;
          break;
        case L'\'':
          replacement = nextSingleQuoteOpen ? L"‘" : L"’";
          nextSingleQuoteOpen = !nextSingleQuoteOpen;
          break;
        default:
          break;
      }
    }

    if (chinesePunct && !input.curlyPunct && ch == L'"') {
      replacement = L"\uff02";
    } else if (chinesePunct && !input.curlyPunct && ch == L'\'') {
      replacement = L"\uff07";
    }

    if (replacement.empty() && input.symbolFullwidth && ((ch >= 0x21 && ch <= 0x2f) || (ch >= 0x3a && ch <= 0x40) ||
         (ch >= 0x5b && ch <= 0x60) || (ch >= 0x7b && ch <= 0x7e))) {
      replacement.push_back(static_cast<wchar_t>(0xff01 + (ch - 0x21)));
    }

    if (replacement.empty() && input.numberFullwidth && ch >= L'0' && ch <= L'9') {
      replacement.push_back(static_cast<wchar_t>(0xff10 + (ch - L'0')));
    }

    if (replacement.empty() && fullShape) {
      if (ch >= 0x21 && ch <= 0x7e && !(ch >= L'0' && ch <= L'9')) {
        replacement.push_back(static_cast<wchar_t>(0xff01 + (ch - 0x21)));
      }
    }

    if (replacement.empty()) replacement.push_back(ch);
    converted += replacement;
  }

  return converted;
}
