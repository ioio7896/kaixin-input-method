use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const RAPIDOCR_DIR_NAME: &str = "RapidOCR-3.9.0";
pub(crate) const PYTHON_RUNTIME_DIR_NAME: &str = ".python-runtime";
const RAPIDOCR_VENV_DIR_NAME: &str = ".venv-rapidocr";
pub(crate) const PACKAGE_MANIFEST_NAME: &str = "package_manifest.sha256";
const REQUIRED_ONNX_MODELS: &[&str] = &["PP-OCRv6_det_medium.onnx", "PP-OCRv6_rec_medium.onnx"];
const OPTIONAL_ONNX_MODELS: &[&str] = &["PP-OCRv6_det_small.onnx", "PP-OCRv6_det_int8.onnx"];

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

pub fn cv_crop_helper_path() -> Option<PathBuf> {
    let helper = install_dir()?.join("tools").join("kaixin_cv_crop.py");
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
    let portable_env = python_env_for_venv_site_packages(install_dir);
    match python_unavailable_reason(&portable, &portable_env) {
        None => {
            return Ok(PythonRuntime {
                executable: fs::canonicalize(&portable).unwrap_or(portable),
                env: portable_env,
            });
        }
        Some(reason) => diagnostics.push(format!("{} ({reason})", portable.display())),
    }

    let python = venv_python_path(install_dir);
    let venv_env = Vec::new();
    match python_unavailable_reason(&python, &venv_env) {
        None => {
            return Ok(PythonRuntime {
                executable: fs::canonicalize(&python).unwrap_or(python),
                env: venv_env,
            });
        }
        Some(reason) => diagnostics.push(format!("{} ({reason})", python.display())),
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

pub fn missing_root_message() -> String {
    format!(
        "RapidOCR directory is unavailable: expected .\\{RAPIDOCR_DIR_NAME} inside the installed program directory."
    )
}

fn venv_python_path(install_dir: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        install_dir
            .join(RAPIDOCR_VENV_DIR_NAME)
            .join("Scripts")
            .join("python.exe")
    }
    #[cfg(not(windows))]
    {
        install_dir
            .join(RAPIDOCR_VENV_DIR_NAME)
            .join("bin")
            .join("python")
    }
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

fn python_env_for_venv_site_packages(install_dir: &Path) -> Vec<(String, OsString)> {
    let site_packages = rapidocr_site_packages_path(install_dir);
    if site_packages.is_dir() {
        vec![("PYTHONPATH".to_string(), site_packages.into_os_string())]
    } else {
        Vec::new()
    }
}

fn rapidocr_site_packages_path(install_dir: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        install_dir
            .join(RAPIDOCR_VENV_DIR_NAME)
            .join("Lib")
            .join("site-packages")
    }
    #[cfg(not(windows))]
    {
        install_dir.join(RAPIDOCR_VENV_DIR_NAME).join("lib")
    }
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

struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    bit_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: [0; 64],
            buffer_len: 0,
            bit_len: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.bit_len = self
            .bit_len
            .wrapping_add((data.len() as u64).wrapping_mul(8));

        if self.buffer_len > 0 {
            let take = (64 - self.buffer_len).min(data.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&data[..take]);
            self.buffer_len += take;
            data = &data[take..];
            if self.buffer_len == 64 {
                process_sha256_block(&mut self.state, &self.buffer);
                self.buffer_len = 0;
            }
        }

        while data.len() >= 64 {
            let block: &[u8; 64] = data[..64].try_into().expect("sha256 block len");
            process_sha256_block(&mut self.state, block);
            data = &data[64..];
        }

        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffer_len = data.len();
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        self.buffer[self.buffer_len] = 0x80;
        self.buffer_len += 1;

        if self.buffer_len > 56 {
            self.buffer[self.buffer_len..].fill(0);
            process_sha256_block(&mut self.state, &self.buffer);
            self.buffer_len = 0;
        }

        self.buffer[self.buffer_len..56].fill(0);
        self.buffer[56..64].copy_from_slice(&self.bit_len.to_be_bytes());
        process_sha256_block(&mut self.state, &self.buffer);

        let mut out = [0u8; 32];
        for (chunk, value) in out.chunks_exact_mut(4).zip(self.state) {
            chunk.copy_from_slice(&value.to_be_bytes());
        }
        out
    }
}

fn process_sha256_block(state: &mut [u32; 8], block: &[u8; 64]) {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];

    let mut w = [0u32; 64];
    for (idx, chunk) in block.chunks_exact(4).enumerate().take(16) {
        w[idx] = u32::from_be_bytes(chunk.try_into().expect("sha256 word len"));
    }
    for idx in 16..64 {
        let s0 = w[idx - 15].rotate_right(7) ^ w[idx - 15].rotate_right(18) ^ (w[idx - 15] >> 3);
        let s1 = w[idx - 2].rotate_right(17) ^ w[idx - 2].rotate_right(19) ^ (w[idx - 2] >> 10);
        w[idx] = w[idx - 16]
            .wrapping_add(s0)
            .wrapping_add(w[idx - 7])
            .wrapping_add(s1);
    }

    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];
    let mut e = state[4];
    let mut f = state[5];
    let mut g = state[6];
    let mut h = state[7];

    for idx in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(K[idx])
            .wrapping_add(w[idx]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);
    state[4] = state[4].wrapping_add(e);
    state[5] = state[5].wrapping_add(f);
    state[6] = state[6].wrapping_add(g);
    state[7] = state[7].wrapping_add(h);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vector() {
        let mut hasher = Sha256::new();
        hasher.update(b"abc");
        assert_eq!(
            hex_lower(&hasher.finalize()),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
