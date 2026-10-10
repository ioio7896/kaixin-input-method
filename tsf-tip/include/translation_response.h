#pragma once
#include <string>
#include <string_view>
#include <cctype>

namespace translation_response {
// Validate the whole JSON acknowledgement and read only top-level fields.
// Nested objects or text containing "ok":true cannot spoof acceptance.
class Reader {
 public:
  explicit Reader(std::string_view value) : value_(value) {}
  bool accepted(std::string_view expected, bool hyMt) {
    bool ok = false; bool haveOk = false; bool haveId = false; bool haveProvider = false;
    std::string id, provider;
    if (!take('{')) return false;
    if (!peek('}')) do {
      std::string key;
      if (!text(key) || !take(':')) return false;
      if (key == "ok") { if (haveOk || !boolean(ok)) return false; haveOk = true; }
      else if (key == "request_id") { if (haveId || !text(id)) return false; haveId = true; }
      else if (key == "provider") { if (haveProvider || !text(provider)) return false; haveProvider = true; }
      else if (!skip(0)) return false;
    } while (take(','));
    if (!take('}')) return false;
    space();
    return pos_ == value_.size() && haveOk && ok && haveId && id == expected && (!hyMt || (haveProvider && provider == "hy-mt2"));
  }
 private:
  void space() { while (pos_ < value_.size() && (value_[pos_] == ' ' || value_[pos_] == '\r' || value_[pos_] == '\n' || value_[pos_] == '\t')) ++pos_; }
  bool peek(char ch) { space(); return pos_ < value_.size() && value_[pos_] == ch; }
  bool take(char ch) { if (!peek(ch)) return false; ++pos_; return true; }
  bool literal(std::string_view word) { space(); if (value_.substr(pos_, word.size()) != word) return false; pos_ += word.size(); return true; }
  bool boolean(bool& result) { if (literal("true")) { result = true; return true; } if (literal("false")) { result = false; return true; } return false; }
  bool text(std::string& result) {
    if (!take('"')) return false;
    result.clear();
    while (pos_ < value_.size()) {
      unsigned char ch = static_cast<unsigned char>(value_[pos_++]);
      if (ch == '"') return true;
      if (ch < 32) return false;
      if (ch != '\\') { result.push_back(static_cast<char>(ch)); continue; }
      if (pos_ == value_.size()) return false;
      char escaped = value_[pos_++];
      if (escaped == 'u') {
        unsigned value = 0;
        for (int n = 0; n < 4; ++n) {
          if (pos_ == value_.size()) return false;
          unsigned char digit = static_cast<unsigned char>(value_[pos_++]);
          if (!std::isxdigit(digit)) return false;
          value = value * 16 + (digit <= '9' ? digit - '0' : std::tolower(digit) - 'a' + 10);
        }
        // Identity fields are ASCII; non-ASCII remains distinct from ASCII IDs.
        result.push_back(value < 128 ? static_cast<char>(value) : '?');
      } else {
        const std::string_view codes = "\"\\/bfnrt";
        auto index = codes.find(escaped); if (index == std::string_view::npos) return false;
        result.push_back(std::string_view("\"\\/\b\f\n\r\t")[index]);
      }
    }
    return false;
  }
  bool skip(unsigned depth) {
    if (depth > 16) return false;
    if (peek('"')) { std::string ignored; return text(ignored); }
    if (take('{')) {
      if (!peek('}')) do { std::string key; if (!text(key) || !take(':') || !skip(depth + 1)) return false; } while (take(','));
      return take('}');
    }
    if (take('[')) { if (!peek(']')) do { if (!skip(depth + 1)) return false; } while (take(',')); return take(']'); }
    if (literal("true") || literal("false") || literal("null")) return true;
    space(); const auto start = pos_; take('-');
    if (!take('0')) { if (pos_ == value_.size() || value_[pos_] < '1' || value_[pos_] > '9') return false; while (pos_ < value_.size() && std::isdigit(static_cast<unsigned char>(value_[pos_]))) ++pos_; }
    if (pos_ < value_.size() && value_[pos_] == '.') {
      ++pos_; const auto digits = pos_; while (pos_ < value_.size() && std::isdigit(static_cast<unsigned char>(value_[pos_]))) ++pos_; if (digits == pos_) return false;
    }
    if (pos_ < value_.size() && (value_[pos_] == 'e' || value_[pos_] == 'E')) {
      ++pos_; if (pos_ < value_.size() && (value_[pos_] == '+' || value_[pos_] == '-')) ++pos_;
      const auto digits = pos_; while (pos_ < value_.size() && std::isdigit(static_cast<unsigned char>(value_[pos_]))) ++pos_; if (digits == pos_) return false;
    }
    return pos_ > start;
  }
  std::string_view value_; size_t pos_ = 0;
};
inline bool accepted(std::string_view response, std::string_view id, bool hyMt) { return Reader(response).accepted(id, hyMt); }
}  // namespace translation_response
