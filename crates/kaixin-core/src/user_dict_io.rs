//! Portable SQLite user-dictionary import and export.

use rusqlite::{params, Connection, DatabaseName};
use std::fs;
use std::io;
use std::path::Path;
use std::ptr::NonNull;
use std::time::Duration;
use zeroize::Zeroizing;

const SQLITE_SCHEMA_VERSION: i32 = 1;

pub fn user_dict_encryption_enabled() -> bool {
    cfg!(windows)
}

pub fn user_dict_file_is_encrypted(path: &Path) -> io::Result<bool> {
    crate::encrypted_journal::has_header(path, crate::user_dict::USER_JOURNAL_MAGIC)
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
    let bytes = Zeroizing::new(user_dict_text_to_sqlite_bytes(text)?);
    fs::write(path, &*bytes)
}

pub fn read_user_dict_sqlite_export_text(path: &Path) -> io::Result<String> {
    // Import/export files are explicitly portable plaintext SQLite. The live
    // dictionary reader above never accepts the same representation.
    let bytes = Zeroizing::new(fs::read(path)?);
    user_dict_text_from_sqlite_bytes(&bytes)
}
