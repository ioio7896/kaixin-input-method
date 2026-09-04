use crate::{app_paths, rapidocr_paths, windows_security};
use fs2::FileExt;
use rusqlite::{params, serialize::OwnedData, Connection, DatabaseName};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::time::{SystemTime, UNIX_EPOCH};

const SCREENSHOT_STORE_MAGIC: &[u8] = b"KXSHOT-DPAPI-1\n";

#[derive(Clone, Debug, Default)]
pub struct ScreenshotRecord {
    pub path: PathBuf,
    pub source: String,
    pub target_hwnd: isize,
    pub source_window_title: Option<String>,
    pub monitor_index: Option<i64>,
    pub x: Option<i64>,
    pub y: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
}

pub fn database_path() -> PathBuf {
    app_paths::local_data_dir()
        .unwrap_or_else(|| std::env::temp_dir().join(app_paths::APP_PATH_NAME))
        .join("screenshot_library.sqlite")
}

pub fn record_screenshot(record: &ScreenshotRecord) -> Result<(), String> {
    if record.path.as_os_str().is_empty() {
        return Err("screenshot path is empty".to_string());
    }
    let path = normalize_path(&record.path);
    let (image_width, image_height) = image::image_dimensions(&path)
        .map(|(width, height)| (Some(width as i64), Some(height as i64)))
        .unwrap_or((None, None));
    let sha256 = rapidocr_paths::sha256_file_hex(&path).ok();
    with_store_mut(|conn| {
        conn.execute(
            "INSERT INTO screenshots (
                path, created_ms, source, target_hwnd, source_window_title,
                monitor_index, x, y, width, height, image_width, image_height, sha256,
                ocr_done
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0)
            ON CONFLICT(path) DO UPDATE SET
                source=excluded.source,
                target_hwnd=excluded.target_hwnd,
                source_window_title=COALESCE(excluded.source_window_title, screenshots.source_window_title),
                monitor_index=COALESCE(excluded.monitor_index, screenshots.monitor_index),
                x=COALESCE(excluded.x, screenshots.x),
                y=COALESCE(excluded.y, screenshots.y),
                width=COALESCE(excluded.width, screenshots.width),
                height=COALESCE(excluded.height, screenshots.height),
                image_width=COALESCE(excluded.image_width, screenshots.image_width),
                image_height=COALESCE(excluded.image_height, screenshots.image_height),
                sha256=COALESCE(excluded.sha256, screenshots.sha256)",
            params![
                path.display().to_string(),
                now_millis(),
                record.source.as_str(),
                record.target_hwnd as i64,
                record.source_window_title.as_deref(),
                record.monitor_index,
                record.x,
                record.y,
                record.width,
                record.height,
                image_width,
                image_height,
                sha256,
            ],
        )
        .map_err(|err| format!("record screenshot metadata: {err}"))?;
        Ok(())
    })
}

pub fn update_ocr_text(path: &Path, text: &str) -> Result<(), String> {
    update_text_field(path, "ocr_text", text, true)
}

pub fn update_translation_text(path: &Path, text: &str) -> Result<(), String> {
    update_text_field(path, "translation_text", text, false)
}

fn update_text_field(
    path: &Path,
    field: &str,
    text: &str,
    mark_ocr_done: bool,
) -> Result<(), String> {
    let path = normalize_path(path);
    let path_text = path.display().to_string();
    with_store_mut(|conn| {
        conn.execute(
            "INSERT INTO screenshots (path, created_ms, source, target_hwnd, ocr_done)
             VALUES (?1, ?2, 'external_image', 0, 0)
             ON CONFLICT(path) DO NOTHING",
            params![path_text, now_millis()],
        )
        .map_err(|err| format!("ensure screenshot metadata: {err}"))?;
        let sql = if mark_ocr_done {
            "UPDATE screenshots SET ocr_text=?2, ocr_done=1 WHERE path=?1"
        } else {
            debug_assert_eq!(field, "translation_text");
            "UPDATE screenshots SET translation_text=?2 WHERE path=?1"
        };
        conn.execute(sql, params![path_text, text.trim()])
            .map_err(|err| format!("update screenshot {field}: {err}"))?;
        Ok(())
    })
}

fn with_store_mut<T>(mutate: impl FnOnce(&Connection) -> Result<T, String>) -> Result<T, String> {
    let path = database_path();
    with_store_mut_at(&path, mutate)
}

fn with_store_mut_at<T>(
    path: &Path,
    mutate: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    ensure_parent(path)?;
    let lock_path = path.with_extension("sqlite.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|err| format!("open screenshot library lock: {err}"))?;
    lock.lock_exclusive()
        .map_err(|err| format!("lock screenshot library: {err}"))?;

    let result = (|| {
        let conn = load_store_connection(path)?;
        let value = mutate(&conn)?;
        save_store_connection(path, &conn)?;
        Ok(value)
    })();

    let _ = lock.unlock();
    drop(lock);
    let _ = fs::remove_file(lock_path);
    result
}

fn load_store_connection(path: &Path) -> Result<Connection, String> {
    let mut conn =
        Connection::open_in_memory().map_err(|err| format!("open screenshot memory DB: {err}"))?;
    if path.is_file() {
        let raw = fs::read(path).map_err(|err| format!("read screenshot library: {err}"))?;
        let mut sqlite = decode_store(&raw)?;
        if !sqlite.is_empty() {
            let data = sqlite_owned_data(&sqlite)?;
            conn.deserialize(DatabaseName::Main, data, false)
                .map_err(|err| format!("deserialize screenshot library: {err}"))?;
        }
        zeroize(&mut sqlite);
    }
    init_schema(&conn)?;
    Ok(conn)
}

fn save_store_connection(path: &Path, conn: &Connection) -> Result<(), String> {
    let data = conn
        .serialize(DatabaseName::Main)
        .map_err(|err| format!("serialize screenshot library: {err}"))?;
    let mut sqlite = data.to_vec();
    let encoded = encode_store(&sqlite)?;
    zeroize(&mut sqlite);

    let temp = path.with_extension("sqlite.write.tmp");
    let _ = fs::remove_file(&temp);
    let mut file =
        File::create(&temp).map_err(|err| format!("create screenshot temporary file: {err}"))?;
    file.write_all(&encoded)
        .and_then(|_| file.sync_all())
        .map_err(|err| format!("write screenshot library: {err}"))?;
    drop(file);
    replace_file_atomically(&temp, path)
}

fn encode_store(sqlite: &[u8]) -> Result<Vec<u8>, String> {
    if cfg!(windows) {
        windows_security::dpapi_protect_with_magic(SCREENSHOT_STORE_MAGIC, sqlite)
            .map_err(|err| format!("encrypt screenshot library: {err}"))
    } else {
        Ok(sqlite.to_vec())
    }
}

fn decode_store(raw: &[u8]) -> Result<Vec<u8>, String> {
    if cfg!(windows) {
        if !windows_security::dpapi_blob_has_magic(SCREENSHOT_STORE_MAGIC, raw) {
            return Err("unencrypted screenshot library rejected".to_string());
        }
        windows_security::dpapi_unprotect_with_magic(SCREENSHOT_STORE_MAGIC, raw)
            .map_err(|err| format!("decrypt screenshot library: {err}"))
    } else {
        Ok(raw.to_vec())
    }
}

fn sqlite_owned_data(bytes: &[u8]) -> Result<OwnedData, String> {
    let ptr = unsafe { rusqlite::ffi::sqlite3_malloc64(bytes.len() as u64) }.cast::<u8>();
    let Some(ptr) = NonNull::new(ptr) else {
        return Err("allocate screenshot sqlite buffer".to_string());
    };
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), bytes.len());
        Ok(OwnedData::from_raw_nonnull(ptr, bytes.len()))
    }
}

fn init_schema(conn: &Connection) -> Result<(), String> {
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|err| format!("configure screenshot library: {err}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS screenshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            path TEXT NOT NULL UNIQUE,
            created_ms INTEGER NOT NULL,
            source TEXT NOT NULL,
            target_hwnd INTEGER NOT NULL DEFAULT 0,
            source_window_title TEXT,
            monitor_index INTEGER,
            x INTEGER,
            y INTEGER,
            width INTEGER,
            height INTEGER,
            image_width INTEGER,
            image_height INTEGER,
            sha256 TEXT,
            ocr_done INTEGER NOT NULL DEFAULT 0,
            ocr_text TEXT,
            translation_text TEXT,
            tags TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_screenshots_created_ms ON screenshots(created_ms DESC);
         CREATE INDEX IF NOT EXISTS idx_screenshots_sha256 ON screenshots(sha256);",
    )
    .map_err(|err| format!("initialize screenshot library: {err}"))?;
    Ok(())
}

fn ensure_parent(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("create screenshot library directory: {err}"))?;
    }
    Ok(())
}

#[cfg(windows)]
fn replace_file_atomically(temp: &Path, final_path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let temp: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let final_path: Vec<u16> = final_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe {
        MoveFileExW(
            temp.as_ptr(),
            final_path.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(format!(
            "replace screenshot library: {}",
            io::Error::last_os_error()
        ))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_file_atomically(temp: &Path, final_path: &Path) -> Result<(), String> {
    fs::rename(temp, final_path).map_err(|err| format!("replace screenshot library: {err}"))
}

fn zeroize(bytes: &mut [u8]) {
    for byte in bytes {
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
