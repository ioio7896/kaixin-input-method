#[derive(Clone, Copy, Debug)]
pub struct CorrectionPrefs {
    pub enabled: bool,
    pub level: CorrectionLevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CorrectionLevel {
    Light,
    Medium,
    Strong,
}

impl Default for CorrectionPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            level: CorrectionLevel::Strong,
        }
    }
}

fn parse_bool(value: &str, default: bool) -> bool {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enabled" => true,
        "0" | "false" | "no" | "off" | "disabled" => false,
        _ => default,
    }
}

fn parse_level(value: &str, default: CorrectionLevel) -> CorrectionLevel {
    match value.trim().to_ascii_lowercase().as_str() {
        "light" | "lite" | "low" | "conservative" | "轻" | "轻纠错" => CorrectionLevel::Light,
        "medium" | "normal" | "default" | "mid" | "中" | "中纠错" => CorrectionLevel::Medium,
        "strong" | "high" | "aggressive" | "强" | "强纠错" => CorrectionLevel::Strong,
        _ => default,
    }
}

pub(crate) fn parse_correction_section(text: &str) -> CorrectionPrefs {
    let mut out = CorrectionPrefs::default();
    let mut in_section = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            in_section = line[1..line.len() - 1]
                .trim()
                .eq_ignore_ascii_case("correction");
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        match k.trim().to_ascii_lowercase().as_str() {
            "enabled" => out.enabled = parse_bool(v, out.enabled),
            "level" | "mode" | "strategy" => out.level = parse_level(v, out.level),
            _ => {}
        }
    }
    out
}

pub fn get_correction_prefs() -> CorrectionPrefs {
    crate::runtime_config::snapshot().correction
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_correction_level() {
        let prefs = parse_correction_section("[correction]\nenabled=true\nlevel=strong\n");
        assert!(prefs.enabled);
        assert_eq!(prefs.level, CorrectionLevel::Strong);

        let prefs = parse_correction_section("[correction]\nmode=light\n");
        assert_eq!(prefs.level, CorrectionLevel::Light);
    }
}
