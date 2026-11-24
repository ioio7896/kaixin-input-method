use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::thread;
use std::time::Duration;

pub const PACKAGE_MANIFEST_NAME: &str = "package_manifest.sha256";
pub const RUNTIME_PAYLOAD_MANIFEST_NAME: &str = "current_runtime_payload.txt";
pub const RUNTIME_SUCCESS_MANIFEST_NAME: &str = "successful_runtime_payload.txt";
pub const RUNTIME_PAYLOAD_ROOT_NAME: &str = "runtime";

pub fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{b:02x}");
    }
    Ok(hex)
}

pub fn normalize_lexical(path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut out = PathBuf::new();
    for comp in abs.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn path_within_root(root: &Path, candidate: &Path) -> bool {
    let root_norm = normalize_lexical(root);
    let cand_norm = normalize_lexical(candidate);
    let r = root_norm
        .to_string_lossy()
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    let c = cand_norm
        .to_string_lossy()
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    c == r || c.starts_with(&format!("{r}\\"))
}

pub fn relative_to_root(root: &Path, path: &Path) -> String {
    let root_norm = normalize_lexical(root);
    let path_norm = normalize_lexical(path);
    let r = root_norm.to_string_lossy();
    let p = path_norm.to_string_lossy();
    let r_trim = r.trim_end_matches('\\');
    let prefix = format!("{r_trim}\\");
    if p.len() >= prefix.len() && p[..prefix.len()].eq_ignore_ascii_case(&prefix) {
        p[prefix.len()..].to_string()
    } else {
        p.to_string()
    }
}

pub fn resolve_runtime_payload_root(root: &Path, allow_missing: bool) -> Result<Option<PathBuf>, String> {
    let resolved_root = normalize_lexical(root);
    let manifest_path = resolved_root.join(RUNTIME_PAYLOAD_MANIFEST_NAME);
    if manifest_path.is_file() {
        let rel = fs::read_to_string(&manifest_path)
            .map_err(|e| format!("read {}: {e}", manifest_path.display()))?;
        let rel = rel.trim();
        if !rel.is_empty() {
            let candidate = normalize_lexical(&resolved_root.join(rel));
            if !path_within_root(&resolved_root, &candidate) {
                return Err(format!(
                    "Runtime payload path escapes install root: {}",
                    candidate.display()
                ));
            }
            return Ok(Some(candidate));
        }
    }

    let runtime_root = resolved_root.join(RUNTIME_PAYLOAD_ROOT_NAME);
    if runtime_root.is_dir() {
        let mut dirs: Vec<PathBuf> = fs::read_dir(&runtime_root)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|x| x.path()))
            .filter(|p| {
                p.is_dir()
                    && (p.join("srf_tsf_tip.dll").is_file()
                        || p.join("x64").join("srf_tsf_tip.dll").is_file())
            })
            .collect();
        dirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        if let Some(best) = dirs.into_iter().next() {
            return Ok(Some(best));
        }
    }

    if allow_missing {
        Ok(None)
    } else {
        Err(format!(
            "Could not resolve runtime payload under {}",
            resolved_root.display()
        ))
    }
}

pub fn resolve_runtime_payload_arch_root(
    root: &Path,
    arch: &str,
    allow_missing: bool,
) -> Result<Option<PathBuf>, String> {
    let Some(payload_root) = resolve_runtime_payload_root(root, allow_missing)? else {
        return Ok(None);
    };
    let arch_root = payload_root.join(arch);
    if arch_root.is_dir() {
        return Ok(Some(arch_root));
    }
    if arch.eq_ignore_ascii_case("x64") {
        return Ok(Some(payload_root));
    }
    if allow_missing {
        Ok(None)
    } else {
        Err(format!(
            "Could not resolve {arch} runtime payload under {}",
            payload_root.display()
        ))
    }
}

pub fn resolve_successful_runtime_payload_root(install_root: &Path) -> Option<PathBuf> {
    let marker = install_root.join(RUNTIME_SUCCESS_MANIFEST_NAME);
    let rel = fs::read_to_string(marker).ok()?;
    let rel = rel.trim();
    if rel.is_empty() {
        return None;
    }
    let candidate = normalize_lexical(&install_root.join(rel));
    if path_within_root(install_root, &candidate) && candidate.is_dir() {
        Some(candidate)
    } else {
        None
    }
}

pub fn write_runtime_payload_markers(install_root: &Path, payload_root: &Path) -> Result<(), String> {
    let rel = relative_to_root(install_root, payload_root);
    fs::write(install_root.join(RUNTIME_PAYLOAD_MANIFEST_NAME), rel.as_bytes())
        .map_err(|e| format!("write {RUNTIME_PAYLOAD_MANIFEST_NAME}: {e}"))?;
    fs::write(install_root.join(RUNTIME_SUCCESS_MANIFEST_NAME), rel.as_bytes())
        .map_err(|e| format!("write {RUNTIME_SUCCESS_MANIFEST_NAME}: {e}"))?;
    Ok(())
}

fn try_apply_pending_installer_replacement(
    path: &Path,
    relative: &str,
    expected: &str,
    stop_helpers: &mut dyn FnMut(),
    log: &mut dyn FnMut(&str),
) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let cand = entry.path();
        if !cand.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy().to_ascii_lowercase();
        if !(name_str.starts_with("is-") && name_str.ends_with(".tmp")) {
            continue;
        }
        let Ok(hash) = sha256_file_hex(&cand) else {
            continue;
        };
        if !hash.eq_ignore_ascii_case(expected) {
            continue;
        }
        log(&format!(
            "WARN: applying pending installer replacement for {relative} from {}",
            name.to_string_lossy()
        ));
        stop_helpers();
        if path.exists() {
            let _ = fs::remove_file(path);
        }
        if fs::rename(&cand, path).is_ok() {
            if let Ok(fixed) = sha256_file_hex(path) {
                if fixed.eq_ignore_ascii_case(expected) {
                    log(&format!(
                        "OK: applied pending installer replacement for {relative}"
                    ));
                    return true;
                }
            }
        }
    }
    false
}

fn assert_package_file_hash(
    path: &Path,
    relative: &str,
    expected: &str,
    manifest_path: &Path,
    stop_helpers: &mut dyn FnMut(),
    log: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let max_attempts = 5usize;
    let mut last_hash = String::from("(unavailable)");
    let mut last_err: Option<String> = None;
    for attempt in 1..=max_attempts {
        match sha256_file_hex(path) {
            Ok(actual) => {
                last_err = None;
                if actual.eq_ignore_ascii_case(expected) {
                    return Ok(());
                }
                if try_apply_pending_installer_replacement(
                    path,
                    relative,
                    expected,
                    stop_helpers,
                    log,
                ) {
                    return Ok(());
                }
                last_hash = actual;
                if attempt < max_attempts {
                    log(&format!(
                        "WARN: package hash mismatch attempt={attempt}/{max_attempts} file={relative} expected={expected} actual={last_hash}"
                    ));
                }
            }
            Err(err) => {
                last_err = Some(err.clone());
                if attempt < max_attempts {
                    log(&format!(
                        "WARN: package hash read failed attempt={attempt}/{max_attempts} file={relative} error={err}"
                    ));
                }
            }
        }
        if attempt < max_attempts {
            thread::sleep(Duration::from_millis((200 * attempt) as u64));
        }
    }
    let err_suffix = last_err
        .map(|e| format!("; hash_error={e}"))
        .unwrap_or_default();
    Err(format!(
        "Package integrity check failed: {relative}; expected={expected}; actual={last_hash}; manifest={}{err_suffix}",
        manifest_path.display()
    ))
}

pub fn verify_package_hash_manifest(
    root: &Path,
    core_files: Option<&[String]>,
    stop_helpers: &mut dyn FnMut(),
    log: &mut dyn FnMut(&str),
) -> Result<usize, String> {
    let root_norm = normalize_lexical(root);
    let manifest_path = root_norm.join(PACKAGE_MANIFEST_NAME);
    if !manifest_path.is_file() {
        return Err(format!("Missing required file: {}", manifest_path.display()));
    }

    let mut remaining: HashSet<String> = HashSet::new();
    if let Some(files) = core_files {
        for f in files {
            remaining.insert(f.replace('/', "\\").to_ascii_lowercase());
        }
    }

    let raw = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("read {}: {e}", manifest_path.display()))?;
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let mut verified = 0usize;

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((hash_part, rel_part)) = trimmed.split_once(" *") else {
            return Err(format!("Invalid package manifest line: {trimmed}"));
        };
        if hash_part.len() != 64 || !hash_part.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("Invalid package manifest line: {trimmed}"));
        }
        let expected = hash_part.to_ascii_lowercase();
        let relative = rel_part.replace('/', "\\");
        if relative.contains(':')
            || Path::new(&relative).is_absolute()
            || relative.split('\\').any(|seg| seg == "..")
        {
            return Err(format!("Unsafe package manifest path: {relative}"));
        }
        if core_files.is_some() && !remaining.remove(&relative.to_ascii_lowercase()) {
            continue;
        }
        let path = root_norm.join(&relative);
        if !path_within_root(&root_norm, &path) {
            return Err(format!("Package manifest path escapes package root: {relative}"));
        }
        let check = if !path.is_file() {
            Err(format!("Missing required file: {}", path.display()))
        } else {
            assert_package_file_hash(
                &path,
                &relative,
                &expected,
                &manifest_path,
                stop_helpers,
                log,
            )
        };
        if let Err(err) = check {
            let _ = fs::remove_file(root_norm.join("component_manifest.ini"));
            return Err(err);
        }
        verified += 1;
    }

    if !remaining.is_empty() {
        let mut missing: Vec<String> = remaining.into_iter().collect();
        missing.sort();
        return Err(format!(
            "Missing core files in package integrity manifest: {}",
            missing.join(", ")
        ));
    }
    if verified == 0 {
        return Err(format!(
            "Package integrity manifest is empty: {}",
            manifest_path.display()
        ));
    }

    log(&format!(
        "OK: verified package integrity manifest entries={verified}"
    ));
    Ok(verified)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn verifies_core_files_and_full_manifest_and_detects_corruption() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("kaixin-manifest-{unique}"));
        fs::create_dir_all(&dir).unwrap();

        let core_path = dir.join("tip.dll");
        fs::write(&core_path, b"core-payload").unwrap();
        let core_hash = sha256_file_hex(&core_path).unwrap();
        fs::write(dir.join("component_manifest.ini"), "[components]\n").unwrap();
        fs::write(
            dir.join(PACKAGE_MANIFEST_NAME),
            format!("{core_hash} *tip.dll\n{core_hash} *ocr.dat\n"),
        )
        .unwrap();

        let mut noop = || {};
        let mut logs = Vec::new();
        // Core-only passes even if optional ocr.dat is missing.
        let count = verify_package_hash_manifest(
            &dir,
            Some(&["tip.dll".to_string()]),
            &mut noop,
            &mut |m| logs.push(m.to_string()),
        )
        .unwrap();
        assert_eq!(count, 1);

        // Full check fails because ocr.dat is missing, and removes component_manifest.ini.
        let err = verify_package_hash_manifest(&dir, None, &mut noop, &mut |_| {}).unwrap_err();
        assert!(err.contains("Missing required file"));
        assert!(!dir.join("component_manifest.ini").exists());

        // Missing core entry in manifest fails.
        let err = verify_package_hash_manifest(
            &dir,
            Some(&["x86.dll".to_string()]),
            &mut noop,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(err.contains("Missing core files"));

        // Corrupted core file fails.
        fs::write(&core_path, b"tampered").unwrap();
        let err = verify_package_hash_manifest(
            &dir,
            Some(&["tip.dll".to_string()]),
            &mut noop,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(err.contains("Package integrity check failed"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_parent_traversal_in_manifest() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("kaixin-manifest-trav-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let fake_hash = "0".repeat(64);
        fs::write(
            dir.join(PACKAGE_MANIFEST_NAME),
            format!("{fake_hash} *../outside.bin\n"),
        )
        .unwrap();
        let mut noop = || {};
        let err = verify_package_hash_manifest(&dir, None, &mut noop, &mut |_| {}).unwrap_err();
        assert!(err.contains("Unsafe package manifest path"));
        let _ = fs::remove_dir_all(&dir);
    }
}
