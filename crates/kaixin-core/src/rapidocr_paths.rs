use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const RAPIDOCR_DIR_NAME: &str = "RapidOCRLite";
pub(crate) const PYTHON_RUNTIME_DIR_NAME: &str = ".python-runtime";
const RAPIDOCR_PACKAGES_DIR_NAME: &str = ".python-packages";
pub(crate) const PACKAGE_MANIFEST_NAME: &str = "package_manifest.sha256";
const REQUIRED_ONNX_MODELS: &[&str] = &[
    "PP-OCRv6_det_medium.onnx",
    "PP-OCRv6_det_small.onnx",
    "PP-OCRv6_rec_medium.onnx",
    "PP-OCRv6_rec_small.onnx",
    "ppocrv6_dict.txt",
];
const OPTIONAL_ONNX_MODELS: &[&str] = &["PP-OCRv6_det_int8.onnx"];

#[derive(Clone, Debug)]
pub struct PythonRuntime {
    pub executable: PathBuf,
    pub env: Vec<(String, OsString)>,
}

pub fn install_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .map(|path| fs::canonicalize(&path).unwrap_or(path))
}

pub fn rapidocr_root() -> Option<PathBuf> {
    let dir = install_dir()?.join(RAPIDOCR_DIR_NAME);
    if is_rapidocr_root(&dir) {
        Some(fs::canonicalize(&dir).unwrap_or(dir))
    } else {
        None
    }
}

pub fn is_rapidocr_root(path: &Path) -> bool {
    path.join("python").join("rapidocr").is_dir()
}

pub fn ocr_helper_path() -> Option<PathBuf> {
    let helper = install_dir()?.join("tools").join("kaixin_ocr_engine.py");
    if helper.is_file() {
        Some(fs::canonicalize(&helper).unwrap_or(helper))
    } else {
        None
    }
}

pub fn python_exe(rapidocr_root: Option<&Path>) -> Result<PathBuf, String> {
    Ok(python_runtime(rapidocr_root)?.executable)
}

pub fn python_runtime(rapidocr_root: Option<&Path>) -> Result<PythonRuntime, String> {
    let owned_root;
    let root = match rapidocr_root {
        Some(root) => root,
        None => {
            owned_root = self::rapidocr_root().ok_or_else(missing_root_message)?;
            owned_root.as_path()
        }
    };
    let install_dir = root
        .parent()
        .ok_or_else(|| format!("RapidOCR path has no install parent: {}", root.display()))?;

    let mut diagnostics = Vec::new();

    let portable = shared_python_runtime_path(install_dir);
    let portable_env = python_env_for_rapidocr_packages(install_dir);
    match python_unavailable_reason(&portable, &portable_env) {
        None => {
            return Ok(PythonRuntime {
                executable: fs::canonicalize(&portable).unwrap_or(portable),
                env: portable_env,
            });
        }
        Some(reason) => diagnostics.push(format!("{} ({reason})", portable.display())),
    }

    Err(format!(
        "RapidOCR bundled Python is unavailable. Checked: {}",
        diagnostics.join("; ")
    ))
}

pub fn apply_python_runtime_env(command: &mut Command, runtime: &PythonRuntime) {
    command
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1");
    for (key, value) in &runtime.env {
        command.env(key, value);
    }
}

pub fn validate_rapidocr_models(rapidocr_root: &Path) -> Result<(), String> {
    let install_dir = rapidocr_root.parent().ok_or_else(|| {
        format!(
            "RapidOCR path has no install parent: {}",
            rapidocr_root.display()
        )
    })?;
    let manifest = read_package_manifest(&install_dir.join(PACKAGE_MANIFEST_NAME))?;
    let model_dir = rapidocr_root.join("python").join("rapidocr").join("models");
    for model_name in REQUIRED_ONNX_MODELS.iter().copied().chain(
        OPTIONAL_ONNX_MODELS
            .iter()
            .copied()
            .filter(|name| model_dir.join(name).is_file()),
    ) {
        let model_path = model_dir.join(model_name);
        if !model_path.is_file() {
            return Err(format!("RapidOCR model missing: {}", model_path.display()));
        }
        let relative = format!("{RAPIDOCR_DIR_NAME}/python/rapidocr/models/{model_name}");
        let expected = manifest.get(&relative).ok_or_else(|| {
            format!("RapidOCR model hash is missing from {PACKAGE_MANIFEST_NAME}: {relative}")
        })?;
        let actual = sha256_file_hex(&model_path)
            .map_err(|e| format!("hash RapidOCR model {}: {e}", model_path.display()))?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(format!(
                "RapidOCR model hash mismatch: {} expected={} actual={}",
                model_path.display(),
                expected,
                actual
            ));
        }
    }
    Ok(())
}

/// Reuse a successful scan while the package manifest and model metadata match.
/// Explicit package checks still call `validate_rapidocr_models` directly.
pub fn validate_rapidocr_models_cached(rapidocr_root: &Path) -> Result<(), String> {
    static CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    let mut cached = cache
        .lock()
        .map_err(|_| "RapidOCR 模型校验缓存锁定失败。".to_string())?;
    let receipt = crate::app_paths::local_data_dir().map(|dir| dir.join("ocr-model-validation.v1"));
    validate_models_with_cache(rapidocr_root, receipt.as_deref(), &mut cached)
}

fn validate_models_with_cache(
    rapidocr_root: &Path,
    receipt: Option<&Path>,
    cached: &mut Option<String>,
) -> Result<(), String> {
    let Ok(stamp) = rapidocr_model_stamp(rapidocr_root) else {
        return validate_rapidocr_models(rapidocr_root);
    };
    if cached.as_deref() == Some(stamp.as_str()) {
        return Ok(());
    }
    if receipt.is_some_and(|path| {
        let mut contents = Vec::new();
        fs::File::open(path)
            .and_then(|file| file.take(65).read_to_end(&mut contents))
            .is_ok()
            && contents == stamp.as_bytes()
    }) {
        *cached = Some(stamp);
        return Ok(());
    }

    validate_rapidocr_models(rapidocr_root)?;
    if rapidocr_model_stamp(rapidocr_root).as_ref() != Ok(&stamp) {
        return Err("RapidOCR 模型在校验期间发生变化，请重试。".to_string());
    }
    if let Some(path) = receipt {
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        if let Some(dir) = path.parent() {
            if fs::create_dir_all(dir)
                .and_then(|_| fs::write(&temp, &stamp))
                .and_then(|_| fs::rename(&temp, path))
                .is_err()
            {
                let _ = fs::remove_file(temp);
            }
        }
    }
    *cached = Some(stamp);
    Ok(())
}

fn rapidocr_model_stamp(rapidocr_root: &Path) -> Result<String, String> {
    let install_dir = rapidocr_root.parent().ok_or_else(|| {
        format!(
            "RapidOCR path has no install parent: {}",
            rapidocr_root.display()
        )
    })?;
    let model_dir = rapidocr_root.join("python").join("rapidocr").join("models");
    let mut stamp = Sha256::new();
    stamp.update(b"kaixin-rapidocr-models-v1\0");
    stamp.update(rapidocr_root.as_os_str().as_encoded_bytes());
    stamp.update(
        sha256_file_hex(&install_dir.join(PACKAGE_MANIFEST_NAME))
            .map_err(|err| format!("read RapidOCR model manifest: {err}"))?,
    );
    for (name, required) in REQUIRED_ONNX_MODELS
        .iter()
        .map(|name| (*name, true))
        .chain(OPTIONAL_ONNX_MODELS.iter().map(|name| (*name, false)))
    {
        let path = model_dir.join(name);
        let file = match fs::File::open(&path) {
            Ok(file) => file,
            Err(err) if !required && err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(format!("open RapidOCR model {}: {err}", path.display())),
        };
        stamp.update(name);
        update_model_stamp(&file, &mut stamp)
            .map_err(|err| format!("read RapidOCR model metadata {}: {err}", path.display()))?;
    }
    Ok(hex_lower(&stamp.finalize()))
}

fn update_model_stamp(file: &fs::File, stamp: &mut Sha256) -> io::Result<()> {
    let metadata = file.metadata()?;
    stamp.update(metadata.len().to_le_bytes());
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::{
            FileBasicInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO,
        };
        let mut info = std::mem::MaybeUninit::<FILE_BASIC_INFO>::uninit();
        let ok = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle() as isize,
                FileBasicInfo,
                info.as_mut_ptr().cast(),
                std::mem::size_of::<FILE_BASIC_INFO>() as u32,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let info = unsafe { info.assume_init() };
        stamp.update(info.CreationTime.to_le_bytes());
        stamp.update(info.LastWriteTime.to_le_bytes());
        stamp.update(info.ChangeTime.to_le_bytes());
    }
    #[cfg(not(windows))]
    stamp.update(format!("{:?}", metadata.modified()?));
    Ok(())
}

pub fn missing_root_message() -> String {
    format!(
        "RapidOCR directory is unavailable: expected .\\{RAPIDOCR_DIR_NAME} inside the installed program directory."
    )
}

fn shared_python_runtime_path(install_dir: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        install_dir.join(PYTHON_RUNTIME_DIR_NAME).join("python.exe")
    }
    #[cfg(not(windows))]
    {
        install_dir
            .join(PYTHON_RUNTIME_DIR_NAME)
            .join("bin")
            .join("python")
    }
}

fn python_env_for_rapidocr_packages(install_dir: &Path) -> Vec<(String, OsString)> {
    let packages = rapidocr_packages_path(install_dir);
    if packages.is_dir() {
        vec![("PYTHONPATH".to_string(), packages.into_os_string())]
    } else {
        Vec::new()
    }
}

fn rapidocr_packages_path(install_dir: &Path) -> PathBuf {
    install_dir.join(RAPIDOCR_PACKAGES_DIR_NAME)
}

fn python_unavailable_reason(path: &Path, env: &[(String, OsString)]) -> Option<String> {
    if !path.is_file() {
        return Some("file is missing".to_string());
    }
    let mut command = Command::new(path);
    command
        .args(["-c", "import os, sys; sys.exit(0)"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
        .env("PYTHONNOUSERSITE", "1")
        .env("PYTHONDONTWRITEBYTECODE", "1");
    for (key, value) in env {
        command.env(key, value);
    }
    #[cfg(windows)]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    match command.status() {
        Ok(status) if status.success() => None,
        Ok(status) => Some(format!("exited with {status}")),
        Err(err) => Some(err.to_string()),
    }
}

pub(crate) fn read_package_manifest(path: &Path) -> Result<HashMap<String, String>, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("read package integrity manifest {}: {e}", path.display()))?;
    let mut entries = HashMap::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(2, char::is_whitespace);
        let Some(hash) = parts.next() else {
            continue;
        };
        if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!(
                "invalid package manifest hash at line {}",
                index + 1
            ));
        }
        let relative = parts
            .next()
            .map(str::trim)
            .and_then(|value| value.strip_prefix('*').or(Some(value)))
            .map(|value| value.replace('\\', "/"))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("invalid package manifest path at line {}", index + 1))?;
        entries.insert(relative, hash.to_ascii_lowercase());
    }
    Ok(entries)
}

pub(crate) fn sha256_file_hex(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_lower(&hasher.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn model_cache_survives_restart_and_rechecks_changed_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let install = std::env::temp_dir().join(format!(
            "kaixin-model-cache-{}-{unique}",
            std::process::id()
        ));
        let root = install.join(RAPIDOCR_DIR_NAME);
        let models = root.join("python").join("rapidocr").join("models");
        fs::create_dir_all(&models).unwrap();
        let mut manifest = String::new();
        for name in REQUIRED_ONNX_MODELS {
            let file = models.join(name);
            fs::write(&file, b"original").unwrap();
            let hash = sha256_file_hex(&file).unwrap();
            manifest.push_str(&format!(
                "{hash}  RapidOCRLite/python/rapidocr/models/{name}\n"
            ));
        }
        let manifest_path = install.join(PACKAGE_MANIFEST_NAME);
        fs::write(&manifest_path, &manifest).unwrap();
        let receipt = install.join("user-cache").join("validated");
        let mut first = None;
        validate_models_with_cache(&root, Some(&receipt), &mut first).unwrap();
        assert_eq!(
            fs::read_to_string(&receipt).unwrap(),
            first.as_deref().unwrap()
        );

        let mut after_restart = None;
        validate_models_with_cache(&root, Some(&receipt), &mut after_restart).unwrap();
        assert_eq!(after_restart, first);

        manifest.push('\n');
        fs::write(&manifest_path, &manifest).unwrap();
        validate_models_with_cache(&root, Some(&receipt), &mut after_restart).unwrap();
        assert_ne!(after_restart, first);
        assert_eq!(
            fs::read_to_string(&receipt).unwrap(),
            after_restart.as_deref().unwrap()
        );

        fs::write(models.join(REQUIRED_ONNX_MODELS[0]), b"modified").unwrap();
        assert!(
            validate_models_with_cache(&root, Some(&receipt), &mut after_restart)
                .unwrap_err()
                .contains("hash mismatch")
        );
        fs::remove_dir_all(install).unwrap();
    }
}
