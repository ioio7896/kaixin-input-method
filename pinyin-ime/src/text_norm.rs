//! UTF-8 文本与拼音输入规范化（简体界面与拉丁拼音混用）。

/// 去掉 Unicode BOM（常见于 Windows 另存为 UTF-8）。
#[inline]
pub fn strip_bom_str(s: &str) -> &str {
    s.strip_prefix('\u{FEFF}').unwrap_or(s)
}

/// 全角拉丁字母、数字、空格 → 半角，便于拼音解析；保留 ü / 声调数字。
pub fn normalize_pinyin_line(s: &str) -> String {
    let s = strip_bom_str(s);
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\u{3000}' => out.push(' '), // 全角空格
            '\u{FF0C}' => out.push(','),
            '\u{FF0E}' => out.push('.'),
            // 全角数字 FF10..=FF19
            c @ '\u{FF10}'..='\u{FF19}' => {
                out.push(std::char::from_u32(c as u32 - 0xFF10 + b'0' as u32).unwrap_or(c));
            }
            // 全角大写 FF21..=FF3A
            c @ '\u{FF21}'..='\u{FF3A}' => {
                out.push(std::char::from_u32(c as u32 - 0xFF21 + b'A' as u32).unwrap_or(c));
            }
            // 全角小写 FF41..=FF5A
            c @ '\u{FF41}'..='\u{FF5A}' => {
                out.push(std::char::from_u32(c as u32 - 0xFF41 + b'a' as u32).unwrap_or(c));
            }
            _ => out.push(ch),
        }
    }
    out
}
