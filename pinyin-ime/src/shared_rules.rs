use std::path::{Path, PathBuf};

pub const APP_PATH_NAME: &str = "kaixin";
pub const CONFIG_FILE_NAME: &str = "kaixin.ini";
pub const LOG_DIR_NAME: &str = "logs";
pub const ENGINE_PIPE_BASE: &str = r"\\.\pipe\KaixinInput_Engine_V5";
pub const ENGINE_MUTEX_BASE: &str = r"Local\KaixinInput_Engine_Mutex_V5";

pub fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

pub fn normalize_install_root(path: &Path) -> PathBuf {
    let normalized = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    strip_windows_verbatim_prefix(normalized)
}

pub fn stable_path_hash(path: &Path) -> u64 {
    let text = path.to_string_lossy().to_lowercase();
    let mut hash = 1469598103934665603u64;
    for unit in text.encode_utf16() {
        hash ^= unit as u64;
        hash = hash.wrapping_mul(1099511628211u64);
    }
    hash
}

pub fn engine_instance_suffix_for_install_root(path: &Path) -> String {
    format!("{:016x}", stable_path_hash(path))
}

pub fn engine_pipe_name_for_suffix(suffix: &str) -> String {
    if suffix.is_empty() {
        ENGINE_PIPE_BASE.to_string()
    } else {
        format!("{ENGINE_PIPE_BASE}_{suffix}")
    }
}

pub fn engine_mutex_name_for_suffix(suffix: &str) -> String {
    if suffix.is_empty() {
        ENGINE_MUTEX_BASE.to_string()
    } else {
        format!("{ENGINE_MUTEX_BASE}_{suffix}")
    }
}

pub fn engine_pipe_name_for_install_root(path: &Path) -> String {
    engine_pipe_name_for_suffix(&engine_instance_suffix_for_install_root(path))
}

pub fn engine_mutex_name_for_install_root(path: &Path) -> String {
    engine_mutex_name_for_suffix(&engine_instance_suffix_for_install_root(path))
}
