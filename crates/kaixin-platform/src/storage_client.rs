use bincode::Options;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;
use zeroize::Zeroize;

pub use kaixin_common::engine_wire::*;

#[derive(Clone, Copy)]
#[repr(u32)]
pub enum StorageDomain {
    Dictionary = 0,
    Clipboard = 1,
}

pub fn capability_path() -> Option<PathBuf> {
    crate::app_paths::local_data_dir().map(|dir| dir.join(CAPABILITY_FILE_NAME))
}

pub fn open_pipe_client(pipe_name: &str) -> Result<std::fs::File, String> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match OpenOptions::new().read(true).write(true).open(pipe_name) {
            Ok(file) => return Ok(file),
            Err(err) if Instant::now() < deadline => {
                let last = err;
                std::thread::sleep(Duration::from_millis(25));
                if Instant::now() >= deadline {
                    return Err(format!("pipe open failed: {last}"));
                }
            }
            Err(err) => return Err(format!("pipe open failed: {err}")),
        }
    }
}

pub fn read_engine_capability_token() -> io::Result<String> {
    let path = capability_path().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "engine capability token path is unavailable",
        )
    })?;
    let bytes = std::fs::read(path)?;
    let mut decoded = if bytes.starts_with(CAPABILITY_MAGIC) {
        crate::windows_security::dpapi_unprotect_with_magic(CAPABILITY_MAGIC, &bytes)?
    } else {
        bytes
    };
    let mut token = match String::from_utf8(std::mem::take(&mut decoded)) {
        Ok(token) => token,
        Err(err) => {
            let mut bytes = err.into_bytes();
            bytes.zeroize();
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid capability token",
            ));
        }
    };
    decoded.zeroize();
    let trimmed = token.trim().to_string();
    token.zeroize();
    let token = trimmed;
    if token.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "empty capability token",
        ))
    } else {
        Ok(token)
    }
}

pub fn request<T: serde::de::DeserializeOwned>(
    domain: StorageDomain,
    request: impl serde::Serialize,
) -> Result<T, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let dir = executable.parent().ok_or("missing engine install path")?;
    let pipe = std::env::var(PIPE_NAME_ENV).unwrap_or_else(|_| {
        crate::shared_rules::engine_pipe_name_for_install_root(
            &crate::shared_rules::normalize_install_root(dir),
        )
    });
    let mut file = match OpenOptions::new().read(true).write(true).open(&pipe) {
        Ok(file) => file,
        Err(error) => {
            let helper = dir.join("srf_ime_engine.exe");
            if error.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) && helper.is_file() {
                std::process::Command::new(helper)
                    .arg("--no-warmup")
                    .spawn()
                    .map_err(|error| format!("start storage service: {error}"))?;
            }
            open_pipe_client(&pipe)?
        }
    };
    let capability =
        zeroize::Zeroizing::new(read_engine_capability_token().map_err(|error| error.to_string())?);
    let mut body = zeroize::Zeroizing::new(Vec::new());
    let units = capability.encode_utf16();
    body.extend_from_slice(&(units.clone().count() as u32).to_le_bytes());
    for unit in units {
        body.extend_from_slice(&unit.to_le_bytes());
    }
    body.extend_from_slice(&(domain as u32).to_le_bytes());
    bincode::serialize_into(&mut *body, &request).map_err(|error| error.to_string())?;
    if body.len() > MAX_STORAGE_REQUEST_BYTES {
        return Err("storage request exceeds the size limit".to_owned());
    }
    let mut header = Vec::with_capacity(12);
    header.extend_from_slice(&PROTOCOL_MAGIC.to_le_bytes());
    header.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
    header.extend_from_slice(&STORAGE_COMMAND.to_le_bytes());
    header.extend_from_slice(&(body.len() as u32).to_le_bytes());
    file.write_all(&header)
        .and_then(|()| file.write_all(&body))
        .map_err(|error| error.to_string())?;
    let mut header = [0; 16];
    file.read_exact(&mut header)
        .map_err(|error| error.to_string())?;
    let magic = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let version = u16::from_le_bytes(header[4..6].try_into().unwrap());
    let command = u16::from_le_bytes(header[6..8].try_into().unwrap());
    let status = i32::from_le_bytes(header[8..12].try_into().unwrap());
    let size = u32::from_le_bytes(header[12..16].try_into().unwrap()) as usize;
    if magic != PROTOCOL_MAGIC
        || version != PROTOCOL_VERSION
        || command != STORAGE_COMMAND
        || size > MAX_STORAGE_RESPONSE_BYTES
    {
        return Err("invalid storage response header".to_owned());
    }
    let mut response = zeroize::Zeroizing::new(vec![0; size]);
    file.read_exact(&mut response)
        .map_err(|error| error.to_string())?;
    if status != 0 {
        return Err(if response.is_empty() {
            format!("storage request rejected ({status})")
        } else {
            String::from_utf8_lossy(&response).into_owned()
        });
    }
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_STORAGE_RESPONSE_BYTES as u64)
        .deserialize(&response)
        .map_err(|error| error.to_string())
}
