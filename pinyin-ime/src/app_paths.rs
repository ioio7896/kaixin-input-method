use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const APP_DISPLAY_NAME: &str = "\u{5f00}\u{5fc3}\u{8f93}\u{5165}\u{6cd5}";
pub use crate::shared_rules::{APP_PATH_NAME, CONFIG_FILE_NAME, LOG_DIR_NAME};

fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn local_data_root() -> Option<PathBuf> {
    non_empty_env_path("LOCALAPPDATA")
        .or_else(|| non_empty_env_path("USERPROFILE").map(|dir| dir.join("AppData").join("Local")))
        .or_else(|| non_empty_env_path("TEMP"))
        .or_else(|| non_empty_env_path("TMP"))
        .or_else(|| Some(std::env::temp_dir()))
}

pub fn local_data_dir() -> Option<PathBuf> {
    local_data_root().map(|dir| dir.join(APP_PATH_NAME))
}

pub fn config_ini_path() -> Option<PathBuf> {
    local_data_dir().map(|dir| dir.join(CONFIG_FILE_NAME))
}

pub fn log_dir() -> Option<PathBuf> {
    local_data_dir().map(|dir| dir.join(LOG_DIR_NAME))
}

pub fn log_file(name: &str) -> Option<PathBuf> {
    log_dir().map(|dir| dir.join(name))
}

/// Reads user-authored configuration text without assuming the legacy file is
/// UTF-8.  Settings written by this application are UTF-8, but older/manual
/// INI files on Chinese Windows are commonly ANSI/GBK or UTF-16LE.  Treating
/// those bytes as UTF-8 used to make a configured Chinese screenshot folder
/// disappear or turn into mojibake when the configuration was saved again.
pub fn read_config_text(path: impl AsRef<Path>) -> io::Result<String> {
    let bytes = fs::read(path)?;
    if let Ok(text) = String::from_utf8(bytes.clone()) {
        return Ok(text.trim_start_matches('\u{feff}').to_string());
    }
    if bytes.starts_with(&[0xff, 0xfe]) {
        return Ok(String::from_utf16_lossy(&utf16_units(&bytes[2..], true)));
    }
    if bytes.starts_with(&[0xfe, 0xff]) {
        return Ok(String::from_utf16_lossy(&utf16_units(&bytes[2..], false)));
    }

    // GBK is the ANSI code page used by Simplified-Chinese Windows.  The
    // decoder also preserves ASCII-only files, so this is a safe final
    // fallback for a non-UTF-8 legacy INI.
    let (text, _, _) = encoding_rs::GBK.decode(&bytes);
    Ok(text.into_owned())
}

fn utf16_units(bytes: &[u8], little_endian: bool) -> Vec<u16> {
    bytes
        .chunks_exact(2)
        .map(|pair| {
            if little_endian {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16le_configuration_text() {
        let bytes = [0xff, 0xfe, b'a', 0, b'=', 0, 0x2d, 0x4e];
        let text = String::from_utf16_lossy(&utf16_units(&bytes[2..], true));
        assert_eq!(text, "a=中");
    }
}
