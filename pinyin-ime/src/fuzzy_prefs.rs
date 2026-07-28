#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FuzzyPairs {
    pub zh_z: bool,
    pub ch_c: bool,
    pub sh_s: bool,
    pub n_l: bool,
    pub f_h: bool,
    pub an_ang: bool,
    pub en_eng: bool,
    pub in_ing: bool,
}

impl Default for FuzzyPairs {
    fn default() -> Self {
        Self {
            zh_z: true,
            ch_c: true,
            sh_s: true,
            n_l: true,
            f_h: true,
            an_ang: true,
            en_eng: true,
            in_ing: true,
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

pub(crate) fn parse_fuzzy_section(text: &str) -> FuzzyPairs {
    let mut out = FuzzyPairs::default();
    let mut in_section = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            in_section = line[1..line.len() - 1].trim().eq_ignore_ascii_case("fuzzy");
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim().to_ascii_lowercase().as_str() {
            "zh_z" | "z_zh" => out.zh_z = parse_bool(value, out.zh_z),
            "ch_c" | "c_ch" => out.ch_c = parse_bool(value, out.ch_c),
            "sh_s" | "s_sh" => out.sh_s = parse_bool(value, out.sh_s),
            "n_l" | "l_n" => out.n_l = parse_bool(value, out.n_l),
            "f_h" | "h_f" => out.f_h = parse_bool(value, out.f_h),
            "an_ang" | "ang_an" => out.an_ang = parse_bool(value, out.an_ang),
            "en_eng" | "eng_en" => out.en_eng = parse_bool(value, out.en_eng),
            "in_ing" | "ing_in" => out.in_ing = parse_bool(value, out.in_ing),
            _ => {}
        }
    }
    out
}

pub fn get_fuzzy_pairs() -> FuzzyPairs {
    crate::runtime_config::snapshot().fuzzy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fuzzy_pairs() {
        let pairs = parse_fuzzy_section("[fuzzy]\nzh_z=false\nf_h=on\nin_ing=0\n");
        assert!(!pairs.zh_z);
        assert!(pairs.f_h);
        assert!(!pairs.in_ing);
    }
}
