//! SQLite-backed user dictionary storage with DPAPI wrapping and cooperative locks.
use fs2::FileExt;
use rusqlite::{params, Connection, DatabaseName};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::atomic::Ordering;
use std::time::{Duration, SystemTime};

const ENCRYPTED_MAGIC: &[u8] = b"KXUD-DPAPI-1\n";
const SQLITE_SCHEMA_VERSION: i32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UserDictResetStamp {
    pub modified: Option<SystemTime>,
    pub len: Option<u64>,
}

fn lock_sidecar(dict_path: &Path) -> PathBuf {
    let mut s = dict_path.as_os_str().to_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

fn staging_path(dict_path: &Path) -> PathBuf {
    let mut s = dict_path.as_os_str().to_os_string();
    s.push(".partial");
    PathBuf::from(s)
}

pub fn user_dict_previous_path(dict_path: &Path) -> PathBuf {
    let mut s = dict_path.as_os_str().to_os_string();
    s.push(".previous");
    PathBuf::from(s)
}

pub fn user_dict_backup_path(dict_path: &Path) -> PathBuf {
    let mut s = dict_path.as_os_str().to_os_string();
    s.push(".bak");
    PathBuf::from(s)
}

pub fn user_dict_reset_marker_path(dict_path: &Path) -> PathBuf {
    let mut s = dict_path.as_os_str().to_os_string();
    s.push(".reset");
    PathBuf::from(s)
}

pub fn user_dict_reset_stamp(dict_path: &Path) -> UserDictResetStamp {
    match fs::metadata(user_dict_reset_marker_path(dict_path)) {
        Ok(meta) => UserDictResetStamp {
            modified: meta.modified().ok(),
            len: Some(meta.len()),
        },
        Err(_) => UserDictResetStamp::default(),
    }
}

fn encryption_enabled() -> bool {
    cfg!(windows)
}

fn is_encrypted_blob(bytes: &[u8]) -> bool {
    bytes.starts_with(ENCRYPTED_MAGIC)
}

pub fn user_dict_encryption_enabled() -> bool {
    cfg!(windows) && encryption_enabled()
}

pub fn user_dict_file_is_encrypted(path: &Path) -> io::Result<bool> {
    fs::read(path).map(|bytes| is_encrypted_blob(&bytes))
}

fn zeroize_bytes(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(Ordering::SeqCst);
}

fn zeroize_vec(bytes: &mut Vec<u8>) {
    zeroize_bytes(bytes.as_mut_slice());
}

#[cfg(windows)]
fn dpapi_protect(data: &[u8]) -> io::Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let ok = unsafe {
        CryptProtectData(
            &input,
            null(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let protected = unsafe {
        let slice = std::slice::from_raw_parts(output.pbData, output.cbData as usize);
        let copied = slice.to_vec();
        let _ = LocalFree(output.pbData.cast());
        copied
    };
    let mut out = Vec::with_capacity(ENCRYPTED_MAGIC.len() + protected.len());
    out.extend_from_slice(ENCRYPTED_MAGIC);
    out.extend_from_slice(&protected);
    Ok(out)
}

#[cfg(windows)]
fn dpapi_unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let payload = data.strip_prefix(ENCRYPTED_MAGIC).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "missing user dictionary encryption header",
        )
    })?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: payload.len() as u32,
        pbData: payload.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            null_mut(),
            null(),
            null(),
            null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let unprotected = unsafe {
        let slice = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let copied = slice.to_vec();
        zeroize_bytes(slice);
        let _ = LocalFree(output.pbData.cast());
        copied
    };
    Ok(unprotected)
}

#[cfg(not(windows))]
fn dpapi_protect(data: &[u8]) -> io::Result<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(not(windows))]
fn dpapi_unprotect(data: &[u8]) -> io::Result<Vec<u8>> {
    Ok(data.to_vec())
}

fn encode_user_dict_contents(contents: &[u8]) -> io::Result<Vec<u8>> {
    if cfg!(windows) && encryption_enabled() {
        dpapi_protect(contents)
    } else {
        Ok(contents.to_vec())
    }
}

pub fn read_user_dict_bytes(path: &Path) -> io::Result<Vec<u8>> {
    let bytes = fs::read(path)?;
    if is_encrypted_blob(&bytes) {
        dpapi_unprotect(&bytes)
    } else if cfg!(windows) {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unencrypted user dictionary rejected",
        ))
    } else {
        Ok(bytes)
    }
}

/// Shared lock used while reading the user dictionary.
pub struct UserDictSharedGuard {
    _file: File,
}

#[derive(Debug)]
struct UserDictSqliteRecord {
    kind: String,
    key1: String,
    key2: String,
    phrase: String,
    freq: u64,
    last_used: u64,
    score: i64,
    pinned: bool,
}

fn initialize_user_dict_sqlite_connection(conn: &Connection) -> io::Result<()> {
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(sqlite_io_error)?;
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS user_dict_entries (
           kind TEXT NOT NULL,
           key1 TEXT NOT NULL DEFAULT '',
           key2 TEXT NOT NULL DEFAULT '',
           phrase TEXT NOT NULL DEFAULT '',
           freq INTEGER NOT NULL DEFAULT 0,
           last_used INTEGER NOT NULL DEFAULT 0,
           score INTEGER NOT NULL DEFAULT 0,
           pinned INTEGER NOT NULL DEFAULT 0,
           sort_order INTEGER NOT NULL DEFAULT 0,
           PRIMARY KEY(kind, key1, key2, phrase)
         );
         CREATE INDEX IF NOT EXISTS idx_user_dict_entries_kind_key
           ON user_dict_entries(kind, key1);
         CREATE INDEX IF NOT EXISTS idx_user_dict_entries_phrase
           ON user_dict_entries(phrase);
         CREATE INDEX IF NOT EXISTS idx_user_dict_entries_last_used
           ON user_dict_entries(last_used);",
    )
    .map_err(sqlite_io_error)?;
    conn.execute_batch(&format!("PRAGMA user_version = {SQLITE_SCHEMA_VERSION};"))
        .map_err(sqlite_io_error)?;
    Ok(())
}

fn sqlite_io_error(err: rusqlite::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

fn parse_sqlite_u64(part: Option<&str>) -> u64 {
    part.unwrap_or_default()
        .trim()
        .parse::<u64>()
        .ok()
        .unwrap_or(0)
}

fn parse_sqlite_i64(part: Option<&str>) -> i64 {
    part.unwrap_or_default()
        .trim()
        .parse::<i64>()
        .ok()
        .unwrap_or(0)
}

fn parse_sqlite_bool(part: Option<&str>) -> bool {
    matches!(
        part.map(str::trim),
        Some("1") | Some("true") | Some("TRUE") | Some("pinned")
    )
}

fn user_dict_sqlite_record_from_line(line: &str) -> Option<UserDictSqliteRecord> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let mut parts = line.split('\t');
    let kind = parts.next()?.trim();
    let record = match kind {
        "I" => UserDictSqliteRecord {
            kind: kind.to_string(),
            key1: parts.next().unwrap_or_default().trim().to_ascii_lowercase(),
            key2: String::new(),
            phrase: parts.next().unwrap_or_default().trim().to_string(),
            freq: parse_sqlite_u64(parts.next()),
            last_used: parse_sqlite_u64(parts.next()),
            score: 0,
            pinned: parse_sqlite_bool(parts.next()),
        },
        "M" | "O" | "N" => UserDictSqliteRecord {
            kind: kind.to_string(),
            key1: parts.next().unwrap_or_default().trim().to_ascii_lowercase(),
            key2: String::new(),
            phrase: parts.next().unwrap_or_default().trim().to_string(),
            freq: parse_sqlite_u64(parts.next()),
            last_used: parse_sqlite_u64(parts.next()),
            score: 0,
            pinned: false,
        },
        "R" => UserDictSqliteRecord {
            kind: kind.to_string(),
            key1: parts.next().unwrap_or_default().trim().to_ascii_lowercase(),
            key2: String::new(),
            phrase: parts.next().unwrap_or_default().trim().to_ascii_lowercase(),
            freq: parse_sqlite_u64(parts.next()),
            last_used: parse_sqlite_u64(parts.next()),
            score: 0,
            pinned: false,
        },
        "B" => {
            let phrase = parts.next().unwrap_or_default().trim().to_string();
            let last_used = parse_sqlite_u64(parts.next());
            if last_used == 0 {
                return None;
            }
            UserDictSqliteRecord {
                kind: kind.to_string(),
                key1: String::new(),
                key2: String::new(),
                phrase,
                freq: 0,
                last_used,
                score: 0,
                pinned: false,
            }
        }
        "S" | "U" => UserDictSqliteRecord {
            kind: kind.to_string(),
            key1: parts.next().unwrap_or_default().trim().to_ascii_lowercase(),
            key2: String::new(),
            phrase: parts.next().unwrap_or_default().trim().to_string(),
            freq: 0,
            score: parse_sqlite_i64(parts.next()),
            last_used: parse_sqlite_u64(parts.next()),
            pinned: false,
        },
        "X" | "Y" => UserDictSqliteRecord {
            kind: kind.to_string(),
            key1: parts.next().unwrap_or_default().trim().to_string(),
            key2: parts.next().unwrap_or_default().trim().to_ascii_lowercase(),
            phrase: parts.next().unwrap_or_default().trim().to_string(),
            freq: 0,
            score: parse_sqlite_i64(parts.next()),
            last_used: parse_sqlite_u64(parts.next()),
            pinned: false,
        },
        "P" => UserDictSqliteRecord {
            kind: kind.to_string(),
            key1: String::new(),
            key2: String::new(),
            phrase: parts.next().unwrap_or_default().trim().to_string(),
            freq: parse_sqlite_u64(parts.next()),
            last_used: parse_sqlite_u64(parts.next()),
            score: 0,
            pinned: false,
        },
        "C" => UserDictSqliteRecord {
            kind: kind.to_string(),
            key1: parts.next().unwrap_or_default().trim().to_string(),
            key2: String::new(),
            phrase: parts.next().unwrap_or_default().trim().to_string(),
            freq: parse_sqlite_u64(parts.next()),
            last_used: parse_sqlite_u64(parts.next()),
            score: 0,
            pinned: false,
        },
        "T" => UserDictSqliteRecord {
            kind: kind.to_string(),
            key1: parts.next().unwrap_or_default().trim().to_string(),
            key2: parts.next().unwrap_or_default().trim().to_string(),
            phrase: parts.next().unwrap_or_default().trim().to_string(),
            freq: parse_sqlite_u64(parts.next()),
            last_used: parse_sqlite_u64(parts.next()),
            score: 0,
            pinned: false,
        },
        "D" => {
            let scope = parts.next().unwrap_or_default().trim();
            UserDictSqliteRecord {
                kind: format!("D:{scope}"),
                key1: parts.next().unwrap_or_default().trim().to_string(),
                key2: parts.next().unwrap_or_default().trim().to_string(),
                phrase: parts.next().unwrap_or_default().trim().to_string(),
                freq: parse_sqlite_u64(parts.next()),
                score: parse_sqlite_i64(parts.next()),
                last_used: parse_sqlite_u64(parts.next()),
                pinned: false,
            }
        }
        "E" => UserDictSqliteRecord {
            kind: kind.to_string(),
            key1: parts.next().unwrap_or_default().trim().to_ascii_lowercase(),
            key2: String::new(),
            phrase: parts.next().unwrap_or_default().trim().to_string(),
            score: parse_sqlite_i64(parts.next()),
            freq: parse_sqlite_u64(parts.next()),
            last_used: parse_sqlite_u64(parts.next()),
            pinned: false,
        },
        _ => return None,
    };
    let has_value = record.kind == "B"
        || record.kind == "S"
        || record.kind == "U"
        || record.kind == "X"
        || record.kind == "Y"
        || record.kind == "E"
        || record.kind.starts_with("D:")
        || record.freq > 0;
    (has_value && !record.phrase.is_empty()).then_some(record)
}

fn line_from_user_dict_sqlite_record(record: &UserDictSqliteRecord) -> Option<String> {
    if let Some(scope) = record.kind.strip_prefix("D:") {
        return Some(format!(
            "D\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            scope,
            record.key1,
            record.key2,
            record.phrase,
            record.freq,
            record.score,
            record.last_used
        ));
    }
    match record.kind.as_str() {
        "I" => Some(format!(
            "I\t{}\t{}\t{}\t{}\t{}",
            record.key1,
            record.phrase,
            record.freq,
            record.last_used,
            u8::from(record.pinned)
        )),
        "M" | "O" | "N" => Some(format!(
            "{}\t{}\t{}\t{}\t{}",
            record.kind, record.key1, record.phrase, record.freq, record.last_used
        )),
        "R" => Some(format!(
            "R\t{}\t{}\t{}\t{}",
            record.key1, record.phrase, record.freq, record.last_used
        )),
        "B" => Some(format!("B\t{}\t{}", record.phrase, record.last_used)),
        "S" | "U" => Some(format!(
            "{}\t{}\t{}\t{}\t{}",
            record.kind, record.key1, record.phrase, record.score, record.last_used
        )),
        "X" | "Y" => Some(format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            record.kind, record.key1, record.key2, record.phrase, record.score, record.last_used
        )),
        "P" => Some(format!(
            "P\t{}\t{}\t{}",
            record.phrase, record.freq, record.last_used
        )),
        "C" => Some(format!(
            "C\t{}\t{}\t{}\t{}",
            record.key1, record.phrase, record.freq, record.last_used
        )),
        "T" => Some(format!(
            "T\t{}\t{}\t{}\t{}\t{}",
            record.key1, record.key2, record.phrase, record.freq, record.last_used
        )),
        "E" => Some(format!(
            "E\t{}\t{}\t{}\t{}\t{}",
            record.key1, record.phrase, record.score, record.freq, record.last_used
        )),
        _ => None,
    }
}

fn sqlite_i64_from_u64(value: u64) -> io::Result<i64> {
    i64::try_from(value).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "value too large"))
}

fn sqlite_i64_from_usize(value: usize) -> io::Result<i64> {
    i64::try_from(value).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "value too large"))
}

fn sqlite_u64_from_i64(value: i64) -> io::Result<u64> {
    u64::try_from(value)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "negative sqlite value"))
}

fn save_user_dict_text_to_connection(conn: &mut Connection, text: &str) -> io::Result<()> {
    let tx = conn.transaction().map_err(sqlite_io_error)?;
    tx.execute("DELETE FROM user_dict_entries", [])
        .map_err(sqlite_io_error)?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT OR REPLACE INTO user_dict_entries
                 (kind, key1, key2, phrase, freq, last_used, score, pinned, sort_order)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .map_err(sqlite_io_error)?;
        for (index, line) in text.lines().enumerate() {
            let Some(record) = user_dict_sqlite_record_from_line(line) else {
                continue;
            };
            stmt.execute(params![
                record.kind,
                record.key1,
                record.key2,
                record.phrase,
                sqlite_i64_from_u64(record.freq)?,
                sqlite_i64_from_u64(record.last_used)?,
                record.score,
                if record.pinned { 1i64 } else { 0i64 },
                sqlite_i64_from_usize(index)?,
            ])
            .map_err(sqlite_io_error)?;
        }
    }
    tx.commit().map_err(sqlite_io_error)
}

fn read_user_dict_text_from_connection(conn: &Connection) -> io::Result<String> {
    let mut stmt = conn
        .prepare(
            "SELECT kind, key1, key2, phrase, freq, last_used, score, pinned
             FROM user_dict_entries
             ORDER BY sort_order, kind, key1, key2, phrase",
        )
        .map_err(sqlite_io_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        })
        .map_err(sqlite_io_error)?;

    let mut lines = Vec::new();
    for row in rows {
        let (kind, key1, key2, phrase, freq, last_used, score, pinned) =
            row.map_err(sqlite_io_error)?;
        let record = UserDictSqliteRecord {
            kind,
            key1,
            key2,
            phrase,
            freq: sqlite_u64_from_i64(freq)?,
            last_used: sqlite_u64_from_i64(last_used)?,
            score,
            pinned: pinned != 0,
        };
        if let Some(line) = line_from_user_dict_sqlite_record(&record) {
            lines.push(line);
        }
    }
    Ok(lines.join("\n"))
}

fn sqlite_owned_data_from_bytes(bytes: &[u8]) -> io::Result<rusqlite::serialize::OwnedData> {
    let ptr = unsafe { rusqlite::ffi::sqlite3_malloc64(bytes.len() as u64) }.cast::<u8>();
    let Some(ptr) = NonNull::new(ptr) else {
        return Err(io::Error::new(
            io::ErrorKind::OutOfMemory,
            "allocate user dictionary sqlite store",
        ));
    };
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), bytes.len());
        Ok(rusqlite::serialize::OwnedData::from_raw_nonnull(
            ptr,
            bytes.len(),
        ))
    }
}

fn user_dict_text_from_sqlite_bytes(bytes: &[u8]) -> io::Result<String> {
    let mut conn = Connection::open_in_memory().map_err(sqlite_io_error)?;
    if !bytes.is_empty() {
        let data = sqlite_owned_data_from_bytes(bytes)?;
        conn.deserialize(DatabaseName::Main, data, false)
            .map_err(sqlite_io_error)?;
    }
    initialize_user_dict_sqlite_connection(&conn)?;
    read_user_dict_text_from_connection(&conn)
}

fn user_dict_text_to_sqlite_bytes(text: &str) -> io::Result<Vec<u8>> {
    let mut conn = Connection::open_in_memory().map_err(sqlite_io_error)?;
    initialize_user_dict_sqlite_connection(&conn)?;
    save_user_dict_text_to_connection(&mut conn, text)?;
    let data = conn
        .serialize(DatabaseName::Main)
        .map_err(sqlite_io_error)?;
    Ok(data.to_vec())
}

pub fn write_plain_user_dict_sqlite(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut sqlite_bytes = user_dict_text_to_sqlite_bytes(text)?;
    let result = fs::write(path, &sqlite_bytes);
    zeroize_vec(&mut sqlite_bytes);
    result
}

pub fn read_user_dict_sqlite_export_text(path: &Path) -> io::Result<String> {
    // Import/export files are explicitly portable plaintext SQLite. The live
    // dictionary reader above never accepts the same representation.
    let mut bytes = fs::read(path)?;
    let text = user_dict_text_from_sqlite_bytes(&bytes);
    zeroize_vec(&mut bytes);
    text
}

fn read_user_dict_sqlite_text_at(path: &Path) -> io::Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }
    let mut bytes = read_user_dict_bytes(path)?;
    let text = user_dict_text_from_sqlite_bytes(&bytes);
    zeroize_vec(&mut bytes);
    text.map(Some)
}

/// Read the current user dictionary, falling back to the two last known-good
/// snapshots when an interrupted replacement or external corruption leaves
/// the primary file unreadable. The next successful persistence update will
/// atomically replace the damaged primary with the recovered in-memory state.
pub fn read_user_dict_sqlite_text(path: &Path) -> io::Result<Option<String>> {
    match read_user_dict_sqlite_text_at(path) {
        Ok(Some(text)) => return Ok(Some(text)),
        Ok(None) => {}
        Err(primary_error) => {
            for recovery_path in [user_dict_previous_path(path), user_dict_backup_path(path)] {
                if let Ok(Some(text)) = read_user_dict_sqlite_text_at(&recovery_path) {
                    return Ok(Some(text));
                }
            }
            return Err(primary_error);
        }
    }

    for recovery_path in [user_dict_previous_path(path), user_dict_backup_path(path)] {
        if let Ok(Some(text)) = read_user_dict_sqlite_text_at(&recovery_path) {
            return Ok(Some(text));
        }
    }
    Ok(None)
}

pub(crate) fn write_user_dict_sqlite_snapshot_unlocked(
    path: &Path,
    contents: &[u8],
) -> io::Result<()> {
    let text = crate::text_encoding::decode_text_bytes(contents).map_err(|message| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{message}: {}", path.display()),
        )
    })?;
    let mut sqlite_bytes = user_dict_text_to_sqlite_bytes(&text)?;
    let result = replace_user_dict_contents(path, &sqlite_bytes);
    zeroize_vec(&mut sqlite_bytes);
    result
}

pub fn write_user_dict_sqlite_snapshot(path: &Path, contents: &[u8]) -> io::Result<()> {
    let _lock = UserDictExclusiveGuard::new(path)?;
    let _ = backup_existing_user_dict(path);
    write_user_dict_sqlite_snapshot_unlocked(path, contents)
}

pub fn write_user_dict_sqlite_snapshot_checked(
    path: &Path,
    expected_reset: UserDictResetStamp,
    contents: &[u8],
) -> io::Result<bool> {
    let _lock = UserDictExclusiveGuard::new(path)?;
    if user_dict_reset_stamp(path) != expected_reset {
        return Ok(false);
    }
    let _ = backup_existing_user_dict(path);
    write_user_dict_sqlite_snapshot_unlocked(path, contents)?;
    Ok(true)
}

pub fn write_user_dict_sqlite_snapshot_with_reset(
    path: &Path,
    contents: &[u8],
) -> io::Result<UserDictResetStamp> {
    let _lock = UserDictExclusiveGuard::new(path)?;
    let _ = backup_existing_user_dict(path);
    write_user_dict_sqlite_snapshot_unlocked(path, contents)?;
    append_user_dict_reset_marker(path)?;
    Ok(user_dict_reset_stamp(path))
}

impl UserDictSharedGuard {
    pub fn new(dict_path: &Path) -> io::Result<Self> {
        let lock_path = lock_sidecar(dict_path);
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)?;
        file.lock_shared()?;
        Ok(Self { _file: file })
    }
}

/// Exclusive lock used while writing or replacing the user dictionary.
pub struct UserDictExclusiveGuard {
    _file: File,
}

impl UserDictExclusiveGuard {
    pub fn new(dict_path: &Path) -> io::Result<Self> {
        let lock_path = lock_sidecar(dict_path);
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)?;
        file.lock_exclusive()?;
        Ok(Self { _file: file })
    }
}

#[cfg(unix)]
fn rename_replace(src: &Path, dst: &Path) -> io::Result<()> {
    fs::rename(src, dst)
}

#[cfg(windows)]
fn rename_replace(src: &Path, dst: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING};

    let mut src_w: Vec<u16> = src.as_os_str().encode_wide().collect();
    src_w.push(0);
    let mut dst_w: Vec<u16> = dst.as_os_str().encode_wide().collect();
    dst_w.push(0);

    let ok = unsafe { MoveFileExW(src_w.as_ptr(), dst_w.as_ptr(), MOVEFILE_REPLACE_EXISTING) != 0 };
    if ok {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Atomically replace the full contents by writing `.partial` first.
pub fn replace_user_dict_contents(dict_path: &Path, contents: &[u8]) -> io::Result<()> {
    if let Some(parent) = dict_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let staging = staging_path(dict_path);
    let mut encoded = encode_user_dict_contents(contents)?;
    let mut f = fs::File::create(&staging)?;
    let write_result = f.write_all(&encoded).and_then(|_| f.sync_all());
    zeroize_vec(&mut encoded);
    write_result?;
    drop(f);
    match rename_replace(&staging, dict_path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&staging);
            Err(e)
        }
    }
}

// Best-effort recovery point for the last known readable snapshot.
fn backup_existing_user_dict(dict_path: &Path) -> io::Result<()> {
    match fs::metadata(dict_path) {
        Ok(meta) if meta.len() > 0 => {}
        Ok(_) => return Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    }

    let mut contents = match read_user_dict_bytes(dict_path) {
        Ok(bytes) => bytes,
        Err(_) => return Ok(()),
    };
    if contents.is_empty() {
        return Ok(());
    }

    let previous = user_dict_previous_path(dict_path);
    let result = replace_user_dict_contents(&previous, &contents);
    if result.is_ok() {
        let backup = user_dict_backup_path(dict_path);
        let _ = replace_user_dict_contents(&backup, &contents);
    }
    zeroize_vec(&mut contents);
    result
}

/// Replaces the encoded on-disk user dictionary blob while holding the exclusive lock.
pub fn write_user_dict_atomic(dict_path: &Path, contents: &[u8]) -> io::Result<()> {
    let _lock = UserDictExclusiveGuard::new(dict_path)?;
    let _ = backup_existing_user_dict(dict_path);
    replace_user_dict_contents(dict_path, contents)
}

fn append_user_dict_reset_marker(dict_path: &Path) -> io::Result<()> {
    let path = user_dict_reset_marker_path(dict_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    writeln!(file, "{:?}", SystemTime::now())?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_test_path(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "kaixin-{name}-{}-{nonce}.sqlite",
            std::process::id()
        ))
    }

    fn cleanup_test_files(path: &Path) {
        for candidate in [
            path.to_path_buf(),
            staging_path(path),
            lock_sidecar(path),
            user_dict_previous_path(path),
            user_dict_backup_path(path),
            user_dict_reset_marker_path(path),
        ] {
            let _ = fs::remove_file(candidate);
        }
    }

    #[test]
    fn unreadable_primary_recovers_previous_readable_snapshot() {
        let path = unique_test_path("interrupted-recovery");
        write_user_dict_sqlite_snapshot(
            &path,
            b"I\tceshilujing\t\xe6\xb5\x8b\xe8\xaf\x95\xe8\xb7\xaf\xe5\xbe\x84\t3\t1\t0",
        )
        .expect("write first snapshot");
        write_user_dict_sqlite_snapshot(&path, b"I\tnewpath\t\xe6\x96\xb0\xe8\xaf\x8d\t1\t2\t0")
            .expect("write second snapshot and recovery point");

        fs::write(&path, b"interrupted-and-unreadable").expect("damage primary snapshot");
        let recovered = read_user_dict_sqlite_text(&path)
            .expect("recover readable previous snapshot")
            .expect("recovered snapshot text");
        assert!(recovered.contains("ceshilujing\t\u{6d4b}\u{8bd5}\u{8def}\u{5f84}"));
        assert!(!recovered.contains("newpath"));

        cleanup_test_files(&path);
    }
}
