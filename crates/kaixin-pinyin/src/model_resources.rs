//! Versioned read-only files alongside the installed binaries.
use memmap2::{Mmap, MmapOptions};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::OnceLock;

pub const RESOURCE_DIR: &str = "model-data";
pub const RESOURCE_MANIFEST: &str = "manifest.sha256";
pub const MODEL_FILE: &str = "compiled_model.bin";
pub const SYMBOL_FILE: &str = "symbol_catalog.tsv";
pub const S2T_FILE: &str = "s2t_chars.tsv";
pub const ALIASES_FILE: &str = "pronunciation_aliases.tsv";
pub const EXCLUSIONS_FILE: &str = "pronunciation_exclusions.tsv";
pub const MODEL_MAGIC: &[u8; 8] = b"SRFMD002";
pub const MODEL_SCHEMA_VERSION: u32 = 2;
pub const BIGRAM_ENTRY_BYTES: usize = 16;
const MAX_RESOURCE_BYTES: u64 = 128 * 1024 * 1024;
const FILES: [&str; 5] = [
    MODEL_FILE,
    SYMBOL_FILE,
    S2T_FILE,
    ALIASES_FILE,
    EXCLUSIONS_FILE,
];

struct ResourceSet {
    directory: PathBuf,
    hashes: HashMap<String, String>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

fn load_set() -> io::Result<ResourceSet> {
    let directory = match std::env::var_os("KAIXIN_MODEL_DIR") {
        Some(path) => PathBuf::from(path),
        None => std::env::current_exe()?
            .ancestors()
            .skip(1)
            .map(|parent| parent.join(RESOURCE_DIR))
            .find(|directory| directory.join(RESOURCE_MANIFEST).is_file())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "model-data/manifest.sha256"))?,
    };
    let mut hashes = HashMap::new();
    for line in fs::read_to_string(directory.join(RESOURCE_MANIFEST))?.lines() {
        let (digest, name) = line
            .split_once('\t')
            .ok_or_else(|| invalid("invalid model manifest row"))?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|ch| ch.is_ascii_hexdigit() && !ch.is_ascii_uppercase())
            || !FILES.contains(&name)
            || hashes.insert(name.to_owned(), digest.to_owned()).is_some()
        {
            return Err(invalid("invalid or duplicate model manifest entry"));
        }
    }
    if hashes.len() != FILES.len() {
        return Err(invalid("incomplete model manifest"));
    }
    Ok(ResourceSet { directory, hashes })
}

fn resource_set() -> io::Result<&'static ResourceSet> {
    static SET: OnceLock<Result<ResourceSet, String>> = OnceLock::new();
    SET.get_or_init(|| load_set().map_err(|err| err.to_string()))
        .as_ref()
        .map_err(|err| invalid(err.clone()))
}

pub struct MappedResource {
    bytes: Mmap,
    _file: File,
}

impl Deref for MappedResource {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

pub fn open_resource(name: &str) -> io::Result<MappedResource> {
    let set = resource_set()?;
    let digest = set
        .hashes
        .get(name)
        .ok_or_else(|| invalid("unknown model resource"))?;
    let path = set.directory.join(format!("{digest}-{name}"));
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_DELETE. Publishers can replace a path
        // without changing the file object that readers mapped.
        options.share_mode(0x0000_0005);
    }
    let file = options.open(path)?;
    let len = file.metadata()?.len();
    if len == 0 || len > MAX_RESOURCE_BYTES {
        return Err(invalid(format!("invalid length for model resource {name}")));
    }
    // Resources are published as immutable, content-addressed files. Writers
    // install a new file object and publish the manifest as their last step.
    let mapped = unsafe { MmapOptions::new().map(&file)? };
    if format!("{:x}", Sha256::digest(&mapped[..])) != *digest {
        return Err(invalid(format!(
            "SHA-256 mismatch for model resource {name}"
        )));
    }
    Ok(MappedResource {
        bytes: mapped,
        _file: file,
    })
}

pub fn model_hash() -> &'static str {
    resource_set()
        .ok()
        .and_then(|set| set.hashes.get(MODEL_FILE))
        .map(String::as_str)
        .unwrap_or("unavailable")
}
