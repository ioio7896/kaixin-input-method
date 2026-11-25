use crate::windows_security::{dpapi_protect_with_magic, dpapi_unprotect_with_magic};
use bincode::Options;
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{de::DeserializeOwned, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

const KEY_MAGIC: &[u8] = b"KXJOURNAL-KEY-1\n";
const MAX_ENVELOPE: usize = 4096;
const MAX_RECORD: usize = 256 * 1024 * 1024;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
const MIN_COMPACTION_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RECORDS_PER_KEY: u64 = 8192;

pub struct Journal {
    path: PathBuf,
    magic: [u8; 8],
    file: Option<File>,
    key: LessSafeKey,
    sequence: u64,
    end: u64,
    checkpoint_end: u64,
    #[cfg(windows)]
    _owner: File,
}

impl Journal {
    pub fn open<T: DeserializeOwned>(
        path: &Path,
        magic: [u8; 8],
        mut replay: impl FnMut(T),
    ) -> io::Result<Self> {
        if !cfg!(windows) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "DPAPI journal requires Windows",
            ));
        }
        if let Some(parent) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        #[cfg(windows)]
        let owner = claim_owner(path)?;
        let (mut file, key) = match open_journal_file(path) {
            Ok(mut file) => {
                let key = read_key(&mut file, magic)?;
                (file, key)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let (temp, key, end) = prepare_file(path, magic, None)?;
                publish_file(&temp, path)?;
                let mut file = open_journal_file(path)?;
                file.seek(io::SeekFrom::Start(end))?;
                (file, key)
            }
            Err(error) => return Err(error),
        };
        let mut sequence = 0_u64;
        let mut end = file.stream_position()?;
        let length = file.metadata()?.len();
        let mut checkpoint_end = end;
        while end < length {
            let remaining = length - end;
            if remaining < 4 {
                break;
            }
            let frame_len = read_u32(&mut file)? as usize;
            if frame_len == 0 {
                if rest_is_zero(&mut file, remaining - 4)? {
                    break;
                }
                return Err(invalid("invalid encrypted journal record length"));
            }
            if remaining - 4 < frame_len as u64 {
                break;
            }
            if !(NONCE_BYTES + TAG_BYTES..=MAX_RECORD + NONCE_BYTES + TAG_BYTES)
                .contains(&frame_len)
            {
                return Err(invalid("invalid encrypted journal record length"));
            }
            let mut nonce = [0; NONCE_BYTES];
            file.read_exact(&mut nonce)?;
            let mut data = Zeroizing::new(vec![0; frame_len - NONCE_BYTES]);
            file.read_exact(&mut data)?;
            let decoded = key
                .open_in_place(
                    Nonce::assume_unique_for_key(nonce),
                    Aad::from(record_aad(magic, sequence, frame_len as u32)),
                    &mut data,
                )
                .map_err(|_| invalid("encrypted journal record authentication failed"))?;
            replay(codec().deserialize(decoded).map_err(io::Error::other)?);
            sequence += 1;
            end += 4 + frame_len as u64;
            if sequence == 1 {
                checkpoint_end = end;
            }
        }
        if end < length {
            file.set_len(end)?;
            file.sync_data()?;
        }
        Ok(Self {
            path: path.to_owned(),
            magic,
            file: Some(file),
            key,
            sequence,
            end,
            checkpoint_end,
            #[cfg(windows)]
            _owner: owner,
        })
    }

    fn writable_file(&mut self) -> io::Result<&mut File> {
        if self.file.is_none() {
            let file = open_journal_file(&self.path)?;
            let length = file.metadata()?.len();
            if length < self.end {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "encrypted journal is shorter than the last acknowledged record",
                ));
            }
            if length > self.end {
                file.set_len(self.end)?;
                file.sync_data()?;
            }
            self.file = Some(file);
        }
        Ok(self.file.as_mut().unwrap())
    }

    pub fn append<T: Serialize>(&mut self, value: &T) -> io::Result<()> {
        let bytes = encode(&self.key, self.magic, self.sequence, value)?;
        let end = self.end;
        let file = self.writable_file()?;
        file.seek(io::SeekFrom::Start(end))?;
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_data()) {
            if let Err(rollback_error) = file.set_len(end).and_then(|()| file.sync_data()) {
                // Reopening retries the truncation before the next append.
                self.file = None;
                return Err(rollback_error);
            }
            return Err(error);
        }
        self.sequence += 1;
        self.end += bytes.len() as u64;
        Ok(())
    }

    pub fn replace<T: Serialize>(&mut self, checkpoint: &T) -> io::Result<()> {
        let (temp, key, end) = prepare_file(
            &self.path,
            self.magic,
            Some(&|key| encode(key, self.magic, 0, checkpoint)),
        )?;
        // Keep the lifetime lock, but release the target handle before promotion.
        drop(self.file.take());
        publish_file(&temp, &self.path)?;
        self.key = key;
        self.sequence = 1;
        self.end = end;
        self.checkpoint_end = end;
        Ok(())
    }

    pub fn needs_compaction(&self) -> bool {
        self.sequence >= MAX_RECORDS_PER_KEY
            || self.end.saturating_sub(self.checkpoint_end)
                >= self.checkpoint_end.max(MIN_COMPACTION_BYTES)
    }
}

pub fn has_header(path: &Path, magic: [u8; 8]) -> io::Result<bool> {
    let mut file = File::open(path)?;
    let mut header = [0; 8];
    match file.read_exact(&mut header) {
        Ok(()) => Ok(header == magic),
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(error) => Err(error),
    }
}

fn open_journal_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(path)
}

#[cfg(windows)]
fn claim_owner(path: &Path) -> io::Result<File> {
    use std::fs::TryLockError;
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

    let mut lock_name = path.as_os_str().to_owned();
    lock_name.push(".lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(lock_name)?;
    // A stable file lock also protects the journal across Windows sessions.
    // Keeping this handle open prevents replacing the lock file during compaction.
    file.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "encrypted journal already has an owner",
        ),
        TryLockError::Error(error) => error,
    })?;
    Ok(file)
}

fn codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_RECORD as u64)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn read_u32(file: &mut File) -> io::Result<u32> {
    let mut bytes = [0; 4];
    file.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn rest_is_zero(file: &mut File, mut remaining: u64) -> io::Result<bool> {
    let mut chunk = [0_u8; 4096];
    while remaining > 0 {
        let to_read = remaining.min(chunk.len() as u64) as usize;
        file.read_exact(&mut chunk[..to_read])?;
        if chunk[..to_read].iter().any(|&byte| byte != 0) {
            return Ok(false);
        }
        remaining -= to_read as u64;
    }
    Ok(true)
}

fn read_key(file: &mut File, magic: [u8; 8]) -> io::Result<LessSafeKey> {
    let mut header = [0; 8];
    file.read_exact(&mut header)?;
    if header != magic {
        return Err(invalid("unrecognized encrypted journal"));
    }
    let size = read_u32(file)? as usize;
    if !(KEY_MAGIC.len()..=MAX_ENVELOPE).contains(&size) {
        return Err(invalid("invalid encrypted journal key envelope length"));
    }
    let mut wrapped = vec![0; size];
    file.read_exact(&mut wrapped)?;
    if !wrapped.starts_with(KEY_MAGIC) {
        return Err(invalid("missing DPAPI journal key envelope"));
    }
    let unwrapped = Zeroizing::new(dpapi_unprotect_with_magic(KEY_MAGIC, &wrapped)?);
    if unwrapped.get(..8) != Some(magic.as_slice()) {
        return Err(invalid("journal key envelope domain mismatch"));
    }
    make_key(&unwrapped[8..])
}

fn make_key(raw: &[u8]) -> io::Result<LessSafeKey> {
    UnboundKey::new(&AES_256_GCM, raw)
        .map(LessSafeKey::new)
        .map_err(|_| invalid("invalid journal key"))
}

fn record_aad(magic: [u8; 8], sequence: u64, size: u32) -> [u8; 20] {
    let mut aad = [0; 20];
    aad[..8].copy_from_slice(&magic);
    aad[8..16].copy_from_slice(&sequence.to_le_bytes());
    aad[16..].copy_from_slice(&size.to_le_bytes());
    aad
}

fn encode<T: Serialize>(
    key: &LessSafeKey,
    magic: [u8; 8],
    sequence: u64,
    value: &T,
) -> io::Result<Vec<u8>> {
    let mut body = Zeroizing::new(codec().serialize(value).map_err(io::Error::other)?);
    if body.len() > MAX_RECORD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "encrypted journal record exceeds size limit",
        ));
    }
    let mut nonce = [0; NONCE_BYTES];
    random(&mut nonce)?;
    let frame_len = u32::try_from(NONCE_BYTES + body.len() + TAG_BYTES).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "encrypted journal record exceeds frame limit",
        )
    })?;
    let tag = key
        .seal_in_place_separate_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(record_aad(magic, sequence, frame_len)),
            &mut body,
        )
        .map_err(|_| io::Error::other("journal record encryption failed"))?;
    let mut frame = Vec::with_capacity(4 + frame_len as usize);
    frame.extend_from_slice(&frame_len.to_le_bytes());
    frame.extend_from_slice(&nonce);
    frame.extend_from_slice(&body);
    frame.extend_from_slice(tag.as_ref());
    Ok(frame)
}

fn random(bytes: &mut [u8]) -> io::Result<()> {
    SystemRandom::new()
        .fill(bytes)
        .map_err(|_| io::Error::other("journal random generation failed"))
}

type CheckpointEncoder<'a> = dyn Fn(&LessSafeKey) -> io::Result<Vec<u8>> + 'a;

fn prepare_file(
    path: &Path,
    magic: [u8; 8],
    checkpoint: Option<&CheckpointEncoder<'_>>,
) -> io::Result<(PathBuf, LessSafeKey, u64)> {
    let mut raw = Zeroizing::new([0_u8; 40]);
    raw[..8].copy_from_slice(&magic);
    random(&mut raw[8..])?;
    let key = make_key(&raw[8..])?;
    let wrapped = dpapi_protect_with_magic(KEY_MAGIC, raw.as_ref())?;
    let mut suffix = [0_u8; 8];
    random(&mut suffix)?;
    let temp = path.with_file_name(format!(
        "{}.{}.{:016x}.tmp",
        path.file_name()
            .ok_or_else(|| invalid("missing journal name"))?
            .to_string_lossy(),
        std::process::id(),
        u64::from_le_bytes(suffix)
    ));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let write_result = (|| {
        if !(KEY_MAGIC.len()..=MAX_ENVELOPE).contains(&wrapped.len()) {
            return Err(invalid("invalid encrypted journal key envelope length"));
        }
        file.write_all(&magic)?;
        file.write_all(&(wrapped.len() as u32).to_le_bytes())?;
        file.write_all(&wrapped)?;
        if let Some(encode) = checkpoint {
            file.write_all(&encode(&key)?)?;
        }
        file.sync_all()?;
        file.stream_position()
    })();
    drop(file);
    match write_result {
        Ok(end) => Ok((temp, key, end)),
        Err(error) => {
            let _ = fs::remove_file(&temp);
            Err(error)
        }
    }
}

fn publish_file(temp: &Path, path: &Path) -> io::Result<()> {
    let result = fs::rename(temp, path);
    #[cfg(windows)]
    let result = {
        let mut result = result;
        use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, ERROR_SHARING_VIOLATION};

        for delay_ms in [10, 30, 80] {
            if !result.as_ref().is_err_and(|error| {
                error.kind() == io::ErrorKind::PermissionDenied
                    || error.raw_os_error().is_some_and(|code| {
                        matches!(code as u32, ERROR_LOCK_VIOLATION | ERROR_SHARING_VIOLATION)
                    })
            }) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            result = fs::rename(temp, path);
        }
        result
    };
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn recovery_discards_only_an_incomplete_final_frame() {
        let name = format!(
            "kaixin-journal-{}-{}.kxj",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let path = std::env::temp_dir().join(name);
        let magic = *b"KXTEST01";
        let mut journal = Journal::open::<u64>(&path, magic, |_| {}).unwrap();
        journal.append(&17_u64).unwrap();
        journal.append(&29_u64).unwrap();
        drop(journal);

        let valid = fs::read(&path).unwrap();
        let mut torn = valid.clone();
        torn.extend_from_slice(&40_u32.to_le_bytes());
        torn.extend_from_slice(&[0; 11]);
        fs::write(&path, torn).unwrap();

        let mut replayed = Vec::new();
        let mut reopened = Journal::open(&path, magic, |row: u64| replayed.push(row)).unwrap();
        assert_eq!(replayed, [17, 29]);
        assert_eq!(fs::read(&path).unwrap(), valid);
        reopened.append(&37_u64).unwrap();
        drop(reopened);

        let mut tampered = fs::read(&path).unwrap();
        *tampered.last_mut().unwrap() ^= 1;
        fs::write(&path, &tampered).unwrap();
        let error = Journal::open::<u64>(&path, magic, |_| {}).err().unwrap();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&path).unwrap(), tampered);
        fs::remove_file(path).unwrap();
    }
}
