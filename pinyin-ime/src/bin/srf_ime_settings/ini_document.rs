use super::*;
#[derive(Clone, Default)]
pub(crate) struct IniDoc {
    pub(crate) sections: BTreeMap<String, BTreeMap<String, String>>,
    pub(crate) original_text: String,
    pub(crate) removed_sections: BTreeSet<String>,
    pub(crate) rewritten_sections: BTreeSet<String>,
    pub(crate) removed_keys: BTreeMap<String, BTreeSet<String>>,
    pub(crate) discarded_invalid_lines: bool,
}

impl IniDoc {
    pub(crate) fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .get(&section.to_ascii_lowercase())
            .and_then(|s| s.get(&key.to_ascii_lowercase()))
            .map(String::as_str)
    }

    pub(crate) fn set(&mut self, section: &str, key: &str, value: impl Into<String>) {
        let section = section.to_ascii_lowercase();
        let key = key.to_ascii_lowercase();
        self.removed_sections.remove(&section);
        let mut remove_removed_key_entry = false;
        if let Some(keys) = self.removed_keys.get_mut(&section) {
            keys.remove(&key);
            if keys.is_empty() {
                remove_removed_key_entry = true;
            }
        }
        if remove_removed_key_entry {
            self.removed_keys.remove(&section);
        }
        self.sections
            .entry(section)
            .or_default()
            .insert(key, value.into());
    }

    pub(crate) fn remove_section(&mut self, section: &str) {
        let section = section.to_ascii_lowercase();
        self.sections.remove(&section);
        self.removed_sections.insert(section.clone());
        self.rewritten_sections.insert(section.clone());
        self.removed_keys.remove(&section);
    }

    pub(crate) fn remove_key(&mut self, section: &str, key: &str) {
        let section = section.to_ascii_lowercase();
        let key = key.to_ascii_lowercase();
        if let Some(values) = self.sections.get_mut(&section) {
            values.remove(&key);
        }
        self.removed_keys.entry(section).or_default().insert(key);
    }

    pub(crate) fn retain_section_keys<F>(&mut self, section: &str, mut keep: F)
    where
        F: FnMut(&str) -> bool,
    {
        let section = section.to_ascii_lowercase();
        if let Some(values) = self.sections.get_mut(&section) {
            let mut removed_keys = BTreeSet::new();
            values.retain(|key, _| {
                let should_keep = keep(key);
                if !should_keep {
                    removed_keys.insert(key.to_string());
                }
                should_keep
            });
            if removed_keys.is_empty() {
                self.removed_keys.remove(&section);
            } else {
                self.removed_keys
                    .entry(section)
                    .or_default()
                    .extend(removed_keys);
            }
        }
    }

    pub(crate) fn render(&self) -> String {
        if !self.original_text.is_empty() {
            return self.render_preserving_original();
        }
        self.render_sorted()
    }

    fn render_sorted(&self) -> String {
        let mut out = String::new();
        for (section, values) in &self.sections {
            out.push('[');
            out.push_str(section);
            out.push_str("]\n");
            for (key, value) in values {
                out.push_str(key);
                out.push('=');
                out.push_str(value);
                out.push('\n');
            }
            out.push('\n');
        }
        out
    }

    fn render_preserving_original(&self) -> String {
        let mut out = String::new();
        let mut current_section = String::new();
        let mut section_removed_or_rewritten = false;
        let mut seen_sections = BTreeSet::new();
        let mut seen_keys: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

        for raw in self.original_text.lines() {
            let trimmed = raw.trim().trim_start_matches('\u{FEFF}');
            if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed.len() >= 2 {
                self.append_missing_keys(&mut out, &current_section, &seen_keys);
                current_section = trimmed[1..trimmed.len() - 1].trim().to_ascii_lowercase();
                section_removed_or_rewritten = self.removed_sections.contains(&current_section)
                    || self.rewritten_sections.contains(&current_section);
                if !self.rewritten_sections.contains(&current_section) {
                    seen_sections.insert(current_section.clone());
                }
                if !section_removed_or_rewritten {
                    out.push_str(raw);
                    out.push('\n');
                }
                continue;
            }

            if section_removed_or_rewritten {
                continue;
            }

            let mut wrote_updated_key = false;
            if !current_section.is_empty()
                && !trimmed.is_empty()
                && !trimmed.starts_with('#')
                && !(trimmed.starts_with(';') && current_section != "shortcuts")
            {
                if let Some((key, _)) = trimmed.split_once('=') {
                    let key = key.trim().to_ascii_lowercase();
                    if self.is_removed_key(&current_section, &key) {
                        wrote_updated_key = true;
                    } else if let Some(value) = self
                        .sections
                        .get(&current_section)
                        .and_then(|section| section.get(&key))
                    {
                        out.push_str(key.trim());
                        out.push('=');
                        out.push_str(value);
                        out.push('\n');
                        seen_keys
                            .entry(current_section.clone())
                            .or_default()
                            .insert(key);
                        wrote_updated_key = true;
                    }
                }
            }

            if !wrote_updated_key {
                out.push_str(raw);
                out.push('\n');
            }
        }

        self.append_missing_keys(&mut out, &current_section, &seen_keys);

        for (section, values) in &self.sections {
            if seen_sections.contains(section) || self.removed_sections.contains(section) {
                continue;
            }
            if !out.ends_with("\n\n") {
                if !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
            }
            out.push('[');
            out.push_str(section);
            out.push_str("]\n");
            for (key, value) in values {
                if self.is_removed_key(section, key) {
                    continue;
                }
                out.push_str(key);
                out.push('=');
                out.push_str(value);
                out.push('\n');
            }
        }

        out
    }

    fn append_missing_keys(
        &self,
        out: &mut String,
        section: &str,
        seen_keys: &BTreeMap<String, BTreeSet<String>>,
    ) {
        if section.is_empty()
            || self.removed_sections.contains(section)
            || self.rewritten_sections.contains(section)
        {
            return;
        }
        let Some(values) = self.sections.get(section) else {
            return;
        };
        let seen = seen_keys.get(section);
        let mut appended_any = false;
        for (key, value) in values {
            if seen.is_some_and(|keys| keys.contains(key)) || self.is_removed_key(section, key) {
                continue;
            }
            if !appended_any && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(key);
            out.push('=');
            out.push_str(value);
            out.push('\n');
            appended_any = true;
        }
    }

    fn is_removed_key(&self, section: &str, key: &str) -> bool {
        self.removed_keys
            .get(section)
            .is_some_and(|keys| keys.contains(key))
    }
}

pub(crate) fn load_config(path: &PathBuf) -> Result<(IniDoc, SettingsModel), String> {
    let text = match app_paths::read_config_text(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err.to_string()),
    };
    let mut config = parse_ini(&text);
    let migrated = apply_pre_model_config_migrations(&mut config);
    let mut model = model_from_canonical_config(&config);
    if migrated || config.discarded_invalid_lines || config_needs_canonical_rewrite(&config) {
        let rendered = rendered_config_for_model(&config, &model);
        let _ = write_config_atomically(path, &rendered);
        config = parse_ini(&rendered);
        model = model_from_config(&config);
    }
    Ok((config, model))
}

pub(crate) fn write_config_atomically(path: &Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("kaixin.ini");
    let temp_path = path.with_file_name(format!(".{file_name}.tmp"));
    let mut file = fs::File::create(&temp_path).map_err(|err| err.to_string())?;
    file.write_all(contents.as_bytes())
        .map_err(|err| err.to_string())?;
    file.sync_all().map_err(|err| err.to_string())?;
    drop(file);
    match replace_file_atomically(&temp_path, path) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&temp_path);
            Err(err)
        }
    }
}

#[cfg(windows)]
pub(crate) fn replace_file_atomically(src: &Path, dst: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let src_w: Vec<u16> = src
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let dst_w: Vec<u16> = dst
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ok = unsafe {
        MoveFileExW(
            src_w.as_ptr(),
            dst_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error().to_string())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
pub(crate) fn replace_file_atomically(src: &Path, dst: &Path) -> Result<(), String> {
    fs::rename(src, dst).map_err(|err| err.to_string())
}

pub(crate) fn parse_ini(text: &str) -> IniDoc {
    let mut doc = IniDoc {
        ..IniDoc::default()
    };
    let mut original_text = String::new();
    let mut current = String::new();
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{FEFF}');
        if line.len() > MAX_CONFIG_LINE_BYTES {
            doc.discarded_invalid_lines = true;
            continue;
        }
        if line.is_empty()
            || line.starts_with('#')
            || (line.starts_with(';') && current != "shortcuts")
        {
            original_text.push_str(raw);
            original_text.push('\n');
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            current = line[1..line.len() - 1].trim().to_ascii_lowercase();
            doc.sections.entry(current.clone()).or_default();
            original_text.push_str(raw);
            original_text.push('\n');
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            doc.discarded_invalid_lines = true;
            continue;
        };
        let key = key.trim();
        if current.is_empty() || key.is_empty() {
            doc.discarded_invalid_lines = true;
            continue;
        }
        doc.set(&current, key, value.trim());
        original_text.push_str(raw);
        original_text.push('\n');
    }
    doc.original_text = original_text;
    for e in pinyin_ime::config_schema::ENTRIES {
        if e.section == "general" && e.key == "config_version" {
            continue;
        }
        if doc.get(e.section, e.key).is_none() {
            doc.set(e.section, e.key, e.default);
        }
    }
    doc
}
