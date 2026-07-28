use crate::app_paths;
use std::fs;

const OCR_KEEP_ALIVE_ENV: &str = "KAIXIN_OCR_KEEP_ALIVE";

pub fn ocr_keep_alive_enabled() -> bool {
    env_bool(OCR_KEEP_ALIVE_ENV).unwrap_or_else(|| config_bool("ocr", "keep_alive", true))
}

fn env_bool(name: &str) -> Option<bool> {
    let value = std::env::var(name).ok()?;
    parse_bool(value.trim())
}

fn config_bool(section: &str, key: &str, default: bool) -> bool {
    let Some(path) = app_paths::config_ini_path() else {
        return default;
    };
    let Ok(text) = fs::read_to_string(path) else {
        return default;
    };
    ini_value(&text, section, key)
        .as_deref()
        .and_then(parse_bool)
        .unwrap_or(default)
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enabled" | "enable" | "常驻" | "开" | "开启" => {
            Some(true)
        }
        "0" | "false" | "no" | "off" | "disabled" | "disable" | "oneshot" | "one-shot" | "关"
        | "关闭" => Some(false),
        _ => None,
    }
}

fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in text.lines() {
        let trimmed = line.trim().trim_start_matches('\u{FEFF}');
        if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_section = trimmed[1..trimmed.len() - 1]
                .trim()
                .eq_ignore_ascii_case(section);
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(key) {
            return Some(value.trim().to_string());
        }
    }
    None
}
