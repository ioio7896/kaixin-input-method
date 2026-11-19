use crate::app_paths;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

const OCR_KEEP_ALIVE_ENV: &str = "KAIXIN_OCR_KEEP_ALIVE";

pub fn ocr_keep_alive_enabled() -> bool {
    env_bool(OCR_KEEP_ALIVE_ENV).unwrap_or_else(|| config_bool("ocr", "keep_alive", true))
}

pub fn ocr_language() -> String {
    config_value("ocr", "language").unwrap_or_else(|| "zh".to_string())
}

/// Best-effort persistence for choices made in the standalone OCR window.
/// A failed preference write must never prevent recognition.
pub fn set_ocr_preference(key: &str, value: &str) -> Result<(), String> {
    let path =
        app_paths::config_ini_path().ok_or_else(|| "OCR 配置文件路径不可用。".to_string())?;
    let original = fs::read_to_string(&path).unwrap_or_default();
    let updated = set_ini_value(&original, "ocr", key, value);
    write_atomic(&path, updated.as_bytes())
}

fn env_bool(name: &str) -> Option<bool> {
    let value = std::env::var(name).ok()?;
    parse_bool(value.trim())
}

fn config_bool(section: &str, key: &str, default: bool) -> bool {
    config_value(section, key)
        .as_deref()
        .and_then(parse_bool)
        .unwrap_or(default)
}

fn config_value(section: &str, key: &str) -> Option<String> {
    let path = app_paths::config_ini_path()?;
    let text = fs::read_to_string(path).ok()?;
    ini_value(&text, section, key)
}

fn set_ini_value(text: &str, section: &str, key: &str, value: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut section_start = None;
    let mut section_end = lines.len();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim().trim_start_matches('\u{FEFF}');
        if !(trimmed.starts_with('[') && trimmed.ends_with(']')) {
            continue;
        }
        if section_start.is_some() {
            section_end = index;
            break;
        }
        if trimmed[1..trimmed.len() - 1]
            .trim()
            .eq_ignore_ascii_case(section)
        {
            section_start = Some(index);
        }
    }
    if let Some(start) = section_start {
        for line in lines.iter_mut().take(section_end).skip(start + 1) {
            let Some((name, _)) = line.split_once('=') else {
                continue;
            };
            if name.trim().eq_ignore_ascii_case(key) {
                *line = format!("{key}={value}");
                return lines.join("\r\n") + "\r\n";
            }
        }
        lines.insert(section_end, format!("{key}={value}"));
    } else {
        if !lines.is_empty() && !lines.last().is_some_and(|line| line.is_empty()) {
            lines.push(String::new());
        }
        lines.push(format!("[{section}]"));
        lines.push(format!("{key}={value}"));
    }
    lines.join("\r\n") + "\r\n"
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .ok_or_else(|| "OCR 配置目录不可用。".to_string())?;
    fs::create_dir_all(parent).map_err(|err| format!("创建 OCR 配置目录失败：{err}"))?;
    let result = (|| -> io::Result<()> {
        let (staging, mut file) = loop {
            let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let staging = parent.join(format!(".kaixin-ocr-{}-{nonce}.tmp", std::process::id()));
            match fs::File::create_new(&staging) {
                Ok(file) => break (staging, file),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(err) => return Err(err),
            }
        };
        let written = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        let result = written.and_then(|()| fs::rename(&staging, path));
        if result.is_err() {
            let _ = fs::remove_file(&staging);
        }
        result
    })();
    result.map_err(|err| format!("更新 OCR 配置失败：{err}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_and_replaces_config() {
        let stamp = std::time::SystemTime::UNIX_EPOCH
            .elapsed()
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("kaixin-tool-prefs-{}-{stamp}", std::process::id()));
        let path = dir.join("kaixin.ini");
        write_atomic(&path, b"[ocr]\r\nlanguage=zh_tw\r\n").unwrap();
        write_atomic(&path, b"[ocr]\r\nlanguage=zh\r\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"[ocr]\r\nlanguage=zh\r\n");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
