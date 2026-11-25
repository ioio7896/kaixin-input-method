use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

const RAPIDOCR_DIR_NAME: &str = "RapidOCRLite";
const RAPIDOCR_DLL_NAME: &str = "rapidocrlite.dll";
pub(crate) const PACKAGE_MANIFEST_NAME: &str = "package_manifest.sha256";
pub const REQUIRED_ONNX_MODELS: &[&str] = &[
    "PP-OCRv6_det_medium.onnx",
    "PP-OCRv6_det_small.onnx",
    "PP-OCRv6_rec_medium.onnx",
    "PP-OCRv6_rec_small.onnx",
    "ppocrv6_dict.txt",
];
const OPTIONAL_ONNX_MODELS: &[&str] = &["PP-OCRv6_det_int8.onnx"];

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

pub fn rapidocr_dll_path(rapidocr_root: &Path) -> PathBuf {
    rapidocr_root.join(RAPIDOCR_DLL_NAME)
}

pub fn rapidocr_models_dir(rapidocr_root: &Path) -> PathBuf {
    rapidocr_root.join("models")
}

pub fn is_rapidocr_root(path: &Path) -> bool {
    rapidocr_dll_path(path).is_file() && rapidocr_models_dir(path).is_dir()
}

pub fn validate_rapidocr_models(rapidocr_root: &Path) -> Result<(), String> {
    let install_dir = rapidocr_root.parent().ok_or_else(|| {
        format!(
            "RapidOCR path has no install parent: {}",
            rapidocr_root.display()
        )
    })?;
    let manifest = read_package_manifest(&install_dir.join(PACKAGE_MANIFEST_NAME))?;
    let dll_path = rapidocr_dll_path(rapidocr_root);
    if !dll_path.is_file() {
        return Err(format!(
            "RapidOCRLite native library missing: {}",
            dll_path.display()
        ));
    }
    validate_manifest_file(
        install_dir,
        &manifest,
        &dll_path,
        "RapidOCRLite native library",
    )?;

    let model_dir = rapidocr_models_dir(rapidocr_root);
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
        validate_manifest_file(install_dir, &manifest, &model_path, "RapidOCR model")?;
    }
    Ok(())
}

fn validate_manifest_file(
    install_dir: &Path,
    manifest: &HashMap<String, String>,
    file_path: &Path,
    label: &str,
) -> Result<(), String> {
    let relative = file_path
        .strip_prefix(install_dir)
        .map_err(|_| {
            format!(
                "{label} escapes install root {}: {}",
                install_dir.display(),
                file_path.display()
            )
        })?
        .to_string_lossy()
        .replace('\\', "/");
    let expected = manifest.get(&relative).ok_or_else(|| {
        format!("{label} hash is missing from {PACKAGE_MANIFEST_NAME}: {relative}")
    })?;
    let actual = sha256_file_hex(file_path)
        .map_err(|e| format!("hash {label} {}: {e}", file_path.display()))?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "{label} hash mismatch: {} expected={} actual={}",
            file_path.display(),
            expected,
            actual
        ));
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
    let receipt = crate::app_paths::local_data_dir().map(|dir| dir.join("ocr-model-validation.v2"));
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
    let dll_path = rapidocr_dll_path(rapidocr_root);
    let model_dir = rapidocr_models_dir(rapidocr_root);
    let mut stamp = Sha256::new();
    stamp.update(b"kaixin-rapidocr-models-v2\0");
    stamp.update(rapidocr_root.as_os_str().as_encoded_bytes());
    stamp.update(
        sha256_file_hex(&install_dir.join(PACKAGE_MANIFEST_NAME))
            .map_err(|err| format!("read RapidOCR model manifest: {err}"))?,
    );
    let dll_file = fs::File::open(&dll_path)
        .map_err(|err| format!("open RapidOCRLite library {}: {err}", dll_path.display()))?;
    stamp.update(RAPIDOCR_DLL_NAME);
    update_model_stamp(&dll_file, &mut stamp).map_err(|err| {
        format!(
            "read RapidOCRLite library metadata {}: {err}",
            dll_path.display()
        )
    })?;

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
        "RapidOCR directory is unavailable: expected .\\{RAPIDOCR_DIR_NAME} with rapidocrlite.dll and models inside the installed program directory."
    )
}

#[cfg(windows)]
#[repr(C)]
struct RpocrCpuInfo {
    avx2_compiled: std::ffi::c_int,
    avx512_compiled: std::ffi::c_int,
    neon_compiled: std::ffi::c_int,
    avx2: std::ffi::c_int,
    avx512: std::ffi::c_int,
    neon: std::ffi::c_int,
    threads: std::ffi::c_int,
    active_isa: [u8; 16],
}

#[cfg(windows)]
pub fn probe_native_runtime(rapidocr_root: &Path) -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::FreeLibrary;
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

    let dll_path = rapidocr_dll_path(rapidocr_root);
    if !dll_path.is_file() {
        return Err(format!("RapidOCRLite 原生库不存在：{}", dll_path.display()));
    }
    let wide: Vec<u16> = dll_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let module = unsafe { LoadLibraryW(wide.as_ptr()) };
    if module == 0 {
        return Err(format!(
            "加载 RapidOCRLite 原生库失败（{}）：{}",
            dll_path.display(),
            io::Error::last_os_error()
        ));
    }

    struct LibGuard(isize);
    impl Drop for LibGuard {
        fn drop(&mut self) {
            unsafe {
                FreeLibrary(self.0);
            }
        }
    }
    let _guard = LibGuard(module);

    for symbol in [
        b"rpocr_options_init_ex\0".as_slice(),
        b"rpocr_create\0".as_slice(),
        b"rpocr_destroy\0".as_slice(),
        b"rpocr_recognize\0".as_slice(),
        b"rpocr_result_free\0".as_slice(),
    ] {
        if unsafe { GetProcAddress(module, symbol.as_ptr()) }.is_none() {
            let name = String::from_utf8_lossy(&symbol[..symbol.len() - 1]);
            return Err(format!(
                "RapidOCRLite 原生库缺少必要导出符号 {name}：{}",
                dll_path.display()
            ));
        }
    }

    let summary = unsafe {
        if let Some(proc_fn) = GetProcAddress(module, c"rpocr_query_cpu_info".as_ptr().cast()) {
            let query: unsafe extern "C" fn(*mut RpocrCpuInfo) = std::mem::transmute(proc_fn);
            let mut info = std::mem::zeroed::<RpocrCpuInfo>();
            query(&mut info);
            let nul = info
                .active_isa
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(info.active_isa.len());
            let isa = String::from_utf8_lossy(&info.active_isa[..nul]);
            let isa = if isa.trim().is_empty() {
                "cpu"
            } else {
                isa.trim()
            };
            format!("native {isa}, {} threads", info.threads.max(1))
        } else {
            "native cpu".to_string()
        }
    };
    Ok(summary)
}

#[cfg(not(windows))]
pub fn probe_native_runtime(_rapidocr_root: &Path) -> Result<String, String> {
    Err("RapidOCRLite native probe is only supported on Windows.".to_string())
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

pub fn sha256_file_hex(path: &Path) -> io::Result<String> {
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
        let models = root.join("models");
        fs::create_dir_all(&models).unwrap();
        let mut manifest = String::new();
        let dll = root.join(RAPIDOCR_DLL_NAME);
        fs::write(&dll, b"native-dll").unwrap();
        let dll_hash = sha256_file_hex(&dll).unwrap();
        manifest.push_str(&format!("{dll_hash}  RapidOCRLite/{RAPIDOCR_DLL_NAME}\n"));
        for name in REQUIRED_ONNX_MODELS {
            let file = models.join(name);
            fs::write(&file, b"original").unwrap();
            let hash = sha256_file_hex(&file).unwrap();
            manifest.push_str(&format!("{hash}  RapidOCRLite/models/{name}\n"));
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
