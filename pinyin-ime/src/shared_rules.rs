use std::path::{Path, PathBuf};

pub const APP_PATH_NAME: &str = "kaixin";
pub const CONFIG_FILE_NAME: &str = "kaixin.ini";
pub const LOG_DIR_NAME: &str = "logs";
pub const ENGINE_PIPE_BASE: &str = r"\\.\pipe\KaixinInput_Engine_V6";
pub const ENGINE_MUTEX_BASE: &str = r"Local\KaixinInput_Engine_Mutex_V6";

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

fn engine_scoped_name(base: &str, version: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        format!("{base}_{version}")
    } else {
        format!("{base}_{version}_{suffix}")
    }
}

pub fn engine_pipe_name_for_suffix(suffix: &str) -> String {
    engine_scoped_name(
        ENGINE_PIPE_BASE,
        include_str!("../../VERSION").trim(),
        suffix,
    )
}

pub fn engine_mutex_name_for_suffix(suffix: &str) -> String {
    engine_scoped_name(
        ENGINE_MUTEX_BASE,
        include_str!("../../VERSION").trim(),
        suffix,
    )
}

pub fn engine_health_value_name() -> String {
    format!("EngineHealthV2_{}", include_str!("../../VERSION").trim())
}

#[cfg(test)]
mod engine_namespace_tests {
    use super::*;

    #[test]
    fn distinct_versions_and_installations_have_distinct_engine_names() {
        let base = ENGINE_PIPE_BASE;
        let old = engine_scoped_name(base, "2.0.0", "1234");
        let current = engine_scoped_name(base, "2026.1105.1", "1234");
        assert_ne!(old, current);
        assert_ne!(current, engine_scoped_name(base, "2026.1105.1", "5678"));
        assert_eq!(
            engine_scoped_name(base, "2026.1105.1", ""),
            format!("{base}_2026.1105.1")
        );
    }
}

pub fn engine_pipe_name_for_install_root(path: &Path) -> String {
    engine_pipe_name_for_suffix(&engine_instance_suffix_for_install_root(path))
}

pub fn engine_mutex_name_for_install_root(path: &Path) -> String {
    engine_mutex_name_for_suffix(&engine_instance_suffix_for_install_root(path))
}
