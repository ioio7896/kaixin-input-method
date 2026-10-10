//! Offline, visible Unicode symbols. Independent of optional phrase lexicons.
use std::collections::HashSet;

pub const PAGE_SIZE: usize = 100;
pub const COLUMNS: usize = 10;
pub const ROWS: usize = 10;

#[derive(Debug)]
pub struct Symbol {
    pub category: &'static str,
    pub text: &'static str,
    pub name: &'static str,
    pub code: String,
    search: String,
}

pub fn load() -> Vec<Symbol> {
    include_str!("../../../data/symbol_catalog.tsv")
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 4, "invalid bundled symbol row");
            let code = fields[1]
                .chars()
                .map(|c| format!("U+{:04X}", c as u32))
                .collect::<Vec<_>>()
                .join(" ");
            Symbol {
                category: fields[0],
                text: fields[1],
                name: fields[2],
                search: format!("{} {}", fields.join(" "), code).to_lowercase(),
                code,
            }
        })
        .collect()
}

pub fn categories(symbols: &[Symbol]) -> Vec<&'static str> {
    let mut seen = HashSet::new();
    symbols
        .iter()
        .map(|s| s.category)
        .filter(|category| seen.insert(*category))
        .collect()
}

pub fn filter(symbols: &[Symbol], category: Option<&str>, query: &str) -> Vec<usize> {
    let normalized = query.trim().to_lowercase();
    let exact_code = normalized
        .strip_prefix("u+")
        .and_then(|hex| u32::from_str_radix(hex, 16).ok());
    let mut seen = HashSet::new();
    symbols
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            category.is_none_or(|category| s.category == category)
                && if let Some(code) = exact_code {
                    s.text.chars().any(|c| c as u32 == code)
                } else {
                    normalized
                        .split_whitespace()
                        .all(|token| s.search.contains(token))
                }
                && seen.insert(s.text)
        })
        .map(|(index, _)| index)
        .collect()
}

pub fn page_count(len: usize) -> usize {
    len.div_ceil(PAGE_SIZE).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_covers_professional_and_daily_symbols_without_invisible_keys() {
        let symbols = load();
        assert!(symbols.len() > 4000);
        assert!(categories(&symbols).len() >= 20);
        let mut seen = HashSet::new();
        for s in &symbols {
            assert!(seen.insert((s.category, s.text)));
            assert!(!s.name.is_empty());
            assert!(!s.text.chars().any(|c| c.is_control() || c.is_whitespace()));
        }
        for text in [
            "∫",
            "∑",
            "α",
            "Ω",
            "⇌",
            "SO₄²⁻",
            "㎡",
            "₉",
            "€",
            "♯",
            "𝄞",
            "→",
            "①",
            "─",
            "😀",
        ] {
            assert!(symbols.iter().any(|s| s.text == text), "missing {text}");
        }
    }

    #[test]
    fn searches_chinese_pinyin_english_unicode_and_formulas() {
        let symbols = load();
        for (query, expected) in [
            ("积分", "∫"),
            ("jifen", "∫"),
            ("integral", "∫"),
            ("alpha", "α"),
            ("U+03B1", "α"),
            ("u+1d11e", "𝄞"),
            ("硫酸根", "SO₄²⁻"),
        ] {
            let hits = filter(&symbols, None, query);
            assert!(hits.iter().any(|&i| symbols[i].text == expected), "{query}");
            assert_eq!(
                hits.iter()
                    .map(|&i| symbols[i].text)
                    .collect::<HashSet<_>>()
                    .len(),
                hits.len()
            );
        }
        assert!(filter(&symbols, Some("化学"), "硫酸根").len() == 1);
        assert!(filter(&symbols, Some("货币"), "积分").is_empty());
        assert!(filter(&symbols, None, "不存在的符号名称").is_empty());
    }

    #[test]
    fn pagination_preserves_every_result_and_keeps_exactly_one_hundred_slots() {
        assert_eq!(PAGE_SIZE, ROWS * COLUMNS);
        for len in [0, 1, 99, 100, 101, 1000, 1001] {
            let pages = page_count(len);
            let covered: usize = (0..pages)
                .map(|p| len.saturating_sub(p * PAGE_SIZE).min(PAGE_SIZE))
                .sum();
            assert_eq!(covered, len);
            assert!(pages >= 1);
        }
    }
}
