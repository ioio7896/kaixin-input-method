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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_and_plain_install_roots_have_same_suffix() {
        let plain = PathBuf::from(r"C:\Program Files (x86)\kaixin");
        let verbatim =
            strip_windows_verbatim_prefix(PathBuf::from(r"\\?\C:\Program Files (x86)\kaixin"));
        assert_eq!(verbatim, plain);
        assert_eq!(
            engine_instance_suffix_for_install_root(&plain),
            "70489ba2d8c271ce"
        );
        assert_eq!(
            engine_instance_suffix_for_install_root(&verbatim),
            "70489ba2d8c271ce"
        );
    }

    #[test]
    fn unc_verbatim_prefix_is_normalized() {
        assert_eq!(
            strip_windows_verbatim_prefix(PathBuf::from(r"\\?\UNC\server\share\kaixin")),
            PathBuf::from(r"\\server\share\kaixin")
        );
    }

    #[test]
    fn engine_names_are_derived_from_install_root_suffix() {
        let root = PathBuf::from(r"C:\Program Files (x86)\kaixin");
        assert_eq!(
            engine_pipe_name_for_install_root(&root),
            r"\\.\pipe\KaixinInput_Engine_V5_70489ba2d8c271ce"
        );
        assert_eq!(
            engine_mutex_name_for_install_root(&root),
            r"Local\KaixinInput_Engine_Mutex_V5_70489ba2d8c271ce"
        );
    }
}
