use chrono::Local;
use kaixin_common::config_schema::{CONFIG_VERSION, ENTRIES};
use std::fs;
use std::path::Path;

fn parse_section_header(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2 {
        Some(trimmed[1..trimmed.len() - 1].trim())
    } else {
        None
    }
}

fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with(';') || trimmed.starts_with('#') {
        return None;
    }
    let eq = trimmed.find('=')?;
    let key = trimmed[..eq].trim();
    if key.is_empty() {
        return None;
    }
    let val = trimmed[eq + 1..].trim();
    Some((key, val))
}

fn ini_has_key(lines: &[String], section: &str, key: &str) -> bool {
    let mut in_section = false;
    for line in lines {
        if let Some(sec) = parse_section_header(line) {
            in_section = sec.eq_ignore_ascii_case(section);
            continue;
        }
        if in_section {
            if let Some((k, _)) = split_key_value(line) {
                if k.eq_ignore_ascii_case(key) {
                    return true;
                }
            }
        }
    }
    false
}

fn get_ini_value(lines: &[String], section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in lines {
        if let Some(sec) = parse_section_header(line) {
            in_section = sec.eq_ignore_ascii_case(section);
            continue;
        }
        if in_section {
            if let Some((k, v)) = split_key_value(line) {
                if k.eq_ignore_ascii_case(key) {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

fn set_ini_value(lines: &mut [String], section: &str, key: &str, value: &str) -> bool {
    let mut in_section = false;
    let mut changed = false;
    for line in lines.iter_mut() {
        if let Some(sec) = parse_section_header(line) {
            in_section = sec.eq_ignore_ascii_case(section);
            continue;
        }
        if in_section {
            let matches_key = split_key_value(line)
                .map(|(k, _)| k.eq_ignore_ascii_case(key))
                .unwrap_or(false);
            if matches_key {
                let new_line = format!("{key}={value}");
                if *line != new_line {
                    *line = new_line;
                    changed = true;
                }
            }
        }
    }
    changed
}

fn remove_ini_key(lines: &mut Vec<String>, section: &str, key: &str) -> bool {
    let mut in_section = false;
    let mut changed = false;
    let mut updated = Vec::with_capacity(lines.len());
    for line in lines.iter() {
        if let Some(sec) = parse_section_header(line) {
            in_section = sec.eq_ignore_ascii_case(section);
            updated.push(line.clone());
            continue;
        }
        if in_section {
            let matches_key = split_key_value(line)
                .map(|(k, _)| k.eq_ignore_ascii_case(key))
                .unwrap_or(false);
            if matches_key {
                changed = true;
                continue;
            }
        }
        updated.push(line.clone());
    }
    if changed {
        *lines = updated;
    }
    changed
}

fn add_ini_default_if_missing(
    lines: &mut Vec<String>,
    section: &str,
    key: &str,
    value: &str,
) -> bool {
    if ini_has_key(lines, section, key) {
        return false;
    }
    let mut section_start: Option<usize> = None;
    let mut insert_at = lines.len();
    for (i, line) in lines.iter().enumerate() {
        if let Some(sec) = parse_section_header(line) {
            if sec.eq_ignore_ascii_case(section) {
                section_start = Some(i);
                insert_at = lines.len();
                for (j, next_line) in lines.iter().enumerate().skip(i + 1) {
                    if parse_section_header(next_line).is_some() {
                        insert_at = j;
                        break;
                    }
                }
                break;
            }
        }
    }

    let entry_line = format!("{key}={value}");
    if section_start.is_some() {
        lines.insert(insert_at, entry_line);
    } else {
        if let Some(last) = lines.last() {
            if !last.trim().is_empty() {
                lines.push(String::new());
            }
        }
        lines.push(format!("[{section}]"));
        lines.push(entry_line);
    }
    true
}

fn set_ini_value_or_add(lines: &mut Vec<String>, section: &str, key: &str, value: &str) -> bool {
    if set_ini_value(lines, section, key, value) {
        return true;
    }
    add_ini_default_if_missing(lines, section, key, value)
}

fn get_app_ini_sections(lines: &[String]) -> Vec<String> {
    let mut sections = Vec::new();
    for line in lines {
        if let Some(sec) = parse_section_header(line) {
            if sec.len() >= 4
                && sec[..4].eq_ignore_ascii_case("app:")
                && !sections.iter().any(|s: &String| s.eq_ignore_ascii_case(sec))
            {
                sections.push(sec.to_string());
            }
        }
    }
    sections
}

fn migrate_app_section_alias(
    lines: &mut Vec<String>,
    section: &str,
    canonical_key: &str,
    aliases: &[&str],
) -> bool {
    let mut changed = false;
    if !ini_has_key(lines, section, canonical_key) {
        for alias in aliases {
            if let Some(value) = get_ini_value(lines, section, alias) {
                if set_ini_value_or_add(lines, section, canonical_key, &value) {
                    changed = true;
                }
                break;
            }
        }
    }
    for alias in aliases {
        if remove_ini_key(lines, section, alias) {
            changed = true;
        }
    }
    changed
}

fn save_text_atomically(path: &Path, lines: &[String]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("create config directory {}: {e}", parent.display()))?;
    }
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("kaixin.ini");
    let temp_path = path.with_file_name(format!(".{file_name}.tmp"));
    let mut body = lines.join("\r\n");
    if !body.is_empty() {
        body.push_str("\r\n");
    }
    fs::write(&temp_path, body.as_bytes())
        .map_err(|e| format!("write temp config {}: {e}", temp_path.display()))?;
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&temp_path, path)
        .map_err(|e| format!("replace config {}: {e}", path.display()))?;
    Ok(())
}

fn backup_user_config(
    config_path: &Path,
    install_root: &Path,
    log: &mut dyn FnMut(&str),
) {
    if !config_path.is_file() {
        return;
    }
    let version = fs::read_to_string(install_root.join("VERSION"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    let safe_version: String = version
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let backup_name = format!(
        "{}.backup.{}.{}",
        config_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("kaixin.ini"),
        safe_version,
        stamp
    );
    let backup_path = config_path.with_file_name(backup_name);
    match fs::copy(config_path, &backup_path) {
        Ok(_) => log(&format!(
            "OK: backed up user config before migration: {}",
            backup_path.display()
        )),
        Err(e) => log(&format!(
            "WARN: could not back up user config before migration: {e}"
        )),
    }
}

pub fn migrate_user_config(
    config_path: &Path,
    install_root: &Path,
    log: &mut dyn FnMut(&str),
) -> Result<(), String> {
    if !config_path.is_file() {
        log("SKIP: user config migration; config file does not exist yet");
        return Ok(());
    }

    let raw = fs::read_to_string(config_path)
        .map_err(|e| format!("read user config {}: {e}", config_path.display()))?;
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let mut lines: Vec<String> = raw.lines().map(|l| l.to_string()).collect();
    let mut changed = false;

    let existing_version: usize = get_ini_value(&lines, "general", "config_version")
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);

    if existing_version < 4 {
        let auto_save = get_ini_value(&lines, "screenshot", "auto_save");
        if auto_save
            .as_deref()
            .map(|v| v.trim().is_empty() || v.trim() == "0")
            .unwrap_or(true)
            && set_ini_value(&mut lines, "screenshot", "auto_save", "1")
        {
            changed = true;
            log("OK: migrated screenshot auto_save old default 0 -> 1");
        }
    }

    if existing_version < 5 {
        let soft_budget = get_ini_value(&lines, "engine", "long_lookup_soft_budget_ms");
        if soft_budget
            .as_deref()
            .map(|v| v.trim().is_empty() || v.trim() == "4")
            .unwrap_or(true)
            && set_ini_value(&mut lines, "engine", "long_lookup_soft_budget_ms", "16")
        {
            changed = true;
            log("OK: migrated long_lookup_soft_budget_ms old default 4 -> 16");
        }
    }

    if existing_version < 10 {
        let mut canonicalized = false;
        if remove_ini_key(&mut lines, "style", "candidate_layout_variant") {
            canonicalized = true;
        }
        let app_sections = get_app_ini_sections(&lines);
        let alias_table: &[(&str, &[&str])] = &[
            ("ascii_mode", &["ascii"]),
            ("hide_ui", &["hide"]),
            ("inline_preedit", &["inline"]),
            ("enhanced_position", &["position"]),
            ("candidate_topmost", &["topmost"]),
            ("focus_policy", &["focus"]),
            ("commit_transport", &["commit", "transport"]),
            ("game_profile", &["game", "profile", "candidate_profile"]),
            ("overlay_anchor", &["anchor"]),
            ("overlay_offset_x", &["offset_x"]),
            ("overlay_offset_y", &["offset_y"]),
            ("overlay_scale", &["scale"]),
            ("overlay_monitor", &["monitor"]),
            ("overlay_backend", &["backend"]),
        ];
        for sec in &app_sections {
            for (canonical, aliases) in alias_table {
                if migrate_app_section_alias(&mut lines, sec, canonical, aliases) {
                    canonicalized = true;
                }
            }
        }
        if canonicalized {
            changed = true;
            log("OK: canonicalized legacy app rule aliases and removed obsolete config keys");
        }
    }

    for entry in ENTRIES {
        if add_ini_default_if_missing(&mut lines, entry.section, entry.key, entry.default) {
            changed = true;
        }
    }
    let version_str = CONFIG_VERSION.to_string();
    if set_ini_value_or_add(&mut lines, "general", "config_version", &version_str) {
        changed = true;
    }

    if changed {
        backup_user_config(config_path, install_root, log);
        save_text_atomically(config_path, &lines)?;
        log(&format!(
            "OK: migrated user config defaults and canonicalized legacy keys: {}",
            config_path.display()
        ));
    } else {
        log("OK: user config migration not needed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn migrates_legacy_ini_and_creates_backup() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("kaixin-cfg-mig-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("VERSION"), "2026.1123.1800").unwrap();

        let ini_path = dir.join("kaixin.ini");
        fs::write(
            &ini_path,
            "[general]\r\nconfig_version=3\r\n[screenshot]\r\nauto_save=0\r\n[style]\r\ncandidate_layout_variant=compact\r\n[app:notepad.exe]\r\nascii=1\r\ncommit=tsf\r\n",
        )
        .unwrap();

        let mut logs = Vec::new();
        migrate_user_config(&ini_path, &dir, &mut |m| logs.push(m.to_string())).unwrap();

        let migrated = fs::read_to_string(&ini_path).unwrap();
        let migrated_lines: Vec<String> = migrated.lines().map(|l| l.to_string()).collect();
        assert_eq!(
            get_ini_value(&migrated_lines, "general", "config_version").as_deref(),
            Some(CONFIG_VERSION.to_string().as_str())
        );
        assert_eq!(
            get_ini_value(&migrated_lines, "screenshot", "auto_save").as_deref(),
            Some("1")
        );
        assert!(!ini_has_key(
            &migrated_lines,
            "style",
            "candidate_layout_variant"
        ));
        assert_eq!(
            get_ini_value(&migrated_lines, "app:notepad.exe", "ascii_mode").as_deref(),
            Some("1")
        );
        assert_eq!(
            get_ini_value(&migrated_lines, "app:notepad.exe", "commit_transport").as_deref(),
            Some("tsf")
        );
        assert!(!ini_has_key(&migrated_lines, "app:notepad.exe", "ascii"));

        let backup_exists = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("kaixin.ini.backup.2026.1123.1800.")
            });
        assert!(backup_exists);

        // Second run must be a no-op.
        let mut second_logs = Vec::new();
        migrate_user_config(&ini_path, &dir, &mut |m| second_logs.push(m.to_string())).unwrap();
        assert!(second_logs
            .iter()
            .any(|m| m.contains("user config migration not needed")));

        let _ = fs::remove_dir_all(&dir);
    }
}
