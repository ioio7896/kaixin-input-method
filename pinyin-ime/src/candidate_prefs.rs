//! 从用户配置 INI 读取候选页大小，供引擎做“小输入长度候选整理”时使用。

const DEFAULT_PAGE_SIZE: usize = 9;
const DEFAULT_HORIZONTAL_COUNT: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CandidatePagePrefs {
    pub page_size: usize,
    pub horizontal: bool,
    pub horizontal_count: usize,
}

pub(crate) fn parse_candidate_page_prefs(text: &str) -> CandidatePagePrefs {
    let mut in_style = false;
    let mut out = CandidatePagePrefs {
        page_size: DEFAULT_PAGE_SIZE,
        horizontal: true,
        horizontal_count: DEFAULT_HORIZONTAL_COUNT,
    };
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            let name = line[1..line.len() - 1].trim().to_ascii_lowercase();
            in_style = name == "style";
            continue;
        }
        if !in_style {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let value = v.trim();
        if key.eq_ignore_ascii_case("candidate_page_size") {
            if let Ok(parsed) = value.parse::<usize>() {
                out.page_size = parsed.clamp(3, 9);
            }
        } else if key.eq_ignore_ascii_case("candidate_horizontal") {
            out.horizontal = matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        } else if key.eq_ignore_ascii_case("candidate_horizontal_count") {
            if let Ok(parsed) = value.parse::<usize>() {
                out.horizontal_count = parsed.clamp(1, 9);
            }
        }
    }
    out
}

pub fn get_candidate_page_size() -> usize {
    crate::runtime_config::snapshot().candidate_page_size
}

pub fn get_effective_candidate_page_size() -> usize {
    crate::runtime_config::snapshot().effective_candidate_page_size
}
