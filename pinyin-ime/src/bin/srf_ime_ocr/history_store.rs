use super::*;

pub(super) fn save_ocr_history(text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }
    let _ = clipboard_store::record_text(text);
    let mut entries = load_ocr_history();
    entries.retain(|entry| entry.text != text);
    entries.insert(
        0,
        OcrHistoryEntry {
            when: chrono::Local::now().format("%m-%d %H:%M").to_string(),
            text: text.to_string(),
        },
    );
    entries.truncate(HISTORY_LIMIT);
    write_ocr_history_sqlite(&ocr_history_sqlite_path(), &entries)
}

pub(super) fn load_ocr_history() -> Vec<OcrHistoryEntry> {
    load_ocr_history_sqlite()
}

pub(super) const OCR_HISTORY_MAGIC: &[u8] = b"KXOCR-DPAPI-1\n";
pub(super) const OCR_HISTORY_SCHEMA_VERSION: i32 = 1;

pub(super) fn ocr_history_sqlite_path() -> PathBuf {
    app_paths::local_data_dir()
        .unwrap_or_else(|| std::env::temp_dir().join(app_paths::APP_PATH_NAME))
        .join("ocr_history.sqlite")
}

pub(super) fn ocr_history_encryption_enabled() -> bool {
    cfg!(windows)
}

pub(super) fn load_ocr_history_sqlite() -> Vec<OcrHistoryEntry> {
    let path = ocr_history_sqlite_path();
    if path.is_file() {
        match read_ocr_history_sqlite(&path) {
            Ok(entries) => return entries,
            Err(_) => return Vec::new(),
        }
    }
    Vec::new()
}

pub(super) fn initialize_ocr_history_connection(conn: &Connection) -> Result<(), String> {
    conn.busy_timeout(Duration::from_secs(5))
        .map_err(|e| format!("configure OCR history sqlite: {e}"))?;
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = NORMAL;
         CREATE TABLE IF NOT EXISTS ocr_history (
           id INTEGER PRIMARY KEY,
           sort_order INTEGER NOT NULL,
           when_label TEXT NOT NULL,
           text TEXT NOT NULL,
           created_at INTEGER NOT NULL DEFAULT 0,
           lang TEXT,
           profile TEXT,
           source_hash TEXT,
           action TEXT,
           char_count INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS idx_ocr_history_created_at
           ON ocr_history(created_at);
         CREATE INDEX IF NOT EXISTS idx_ocr_history_text
           ON ocr_history(text);",
    )
    .map_err(|e| format!("initialize OCR history sqlite: {e}"))?;
    conn.execute_batch(&format!(
        "PRAGMA user_version = {OCR_HISTORY_SCHEMA_VERSION};"
    ))
    .map_err(|e| format!("stamp OCR history sqlite: {e}"))?;
    Ok(())
}

pub(super) fn ocr_sqlite_owned_data_from_bytes(
    bytes: &[u8],
) -> Result<rusqlite::serialize::OwnedData, String> {
    let ptr = unsafe { rusqlite::ffi::sqlite3_malloc64(bytes.len() as u64) }.cast::<u8>();
    let Some(ptr) = NonNull::new(ptr) else {
        return Err("allocate OCR history sqlite store".to_string());
    };
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), bytes.len());
        Ok(rusqlite::serialize::OwnedData::from_raw_nonnull(
            ptr,
            bytes.len(),
        ))
    }
}

pub(super) fn decode_ocr_history_store(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if windows_security::dpapi_blob_has_magic(OCR_HISTORY_MAGIC, bytes) {
        windows_security::dpapi_unprotect_with_magic(OCR_HISTORY_MAGIC, bytes)
            .map_err(|e| format!("decrypt OCR history: {e}"))
    } else if cfg!(windows) {
        Err("unencrypted OCR history rejected".to_string())
    } else {
        Ok(bytes.to_vec())
    }
}

pub(super) fn encode_ocr_history_store(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if ocr_history_encryption_enabled() {
        windows_security::dpapi_protect_with_magic(OCR_HISTORY_MAGIC, bytes)
            .map_err(|e| format!("encrypt OCR history: {e}"))
    } else {
        Ok(bytes.to_vec())
    }
}

pub(super) fn read_ocr_history_sqlite(path: &Path) -> Result<Vec<OcrHistoryEntry>, String> {
    let raw = fs::read(path).map_err(|e| format!("read OCR history sqlite: {e}"))?;
    let sqlite_bytes = decode_ocr_history_store(&raw)?;
    let mut conn =
        Connection::open_in_memory().map_err(|e| format!("open OCR history sqlite: {e}"))?;
    if !sqlite_bytes.is_empty() {
        let data = ocr_sqlite_owned_data_from_bytes(&sqlite_bytes)?;
        conn.deserialize(DatabaseName::Main, data, false)
            .map_err(|e| format!("load OCR history sqlite: {e}"))?;
    }
    initialize_ocr_history_connection(&conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT when_label, text
             FROM ocr_history
             ORDER BY sort_order, created_at DESC, id DESC
             LIMIT ?1",
        )
        .map_err(|e| format!("prepare OCR history read: {e}"))?;
    let rows = stmt
        .query_map([HISTORY_LIMIT as i64], |row| {
            Ok(OcrHistoryEntry {
                when: row.get(0)?,
                text: row.get(1)?,
            })
        })
        .map_err(|e| format!("read OCR history rows: {e}"))?;
    let mut entries = Vec::new();
    for row in rows {
        let entry = row.map_err(|e| format!("read OCR history entry: {e}"))?;
        if !entry.text.trim().is_empty() {
            entries.push(entry);
        }
    }
    Ok(entries)
}

pub(super) fn write_ocr_history_sqlite(
    path: &Path,
    entries: &[OcrHistoryEntry],
) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("create OCR history dir: {e}"))?;
    }
    let mut conn =
        Connection::open_in_memory().map_err(|e| format!("open OCR history sqlite: {e}"))?;
    initialize_ocr_history_connection(&conn)?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("begin OCR history transaction: {e}"))?;
    tx.execute("DELETE FROM ocr_history", [])
        .map_err(|e| format!("clear OCR history: {e}"))?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO ocr_history
                 (sort_order, when_label, text, created_at, action, char_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .map_err(|e| format!("prepare OCR history insert: {e}"))?;
        let now = chrono::Local::now().timestamp();
        for (index, entry) in entries.iter().take(HISTORY_LIMIT).enumerate() {
            stmt.execute(params![
                index as i64,
                entry.when.as_str(),
                entry.text.as_str(),
                now.saturating_sub(index as i64),
                "copy",
                entry.text.chars().count() as i64,
            ])
            .map_err(|e| format!("write OCR history entry: {e}"))?;
        }
    }
    tx.commit()
        .map_err(|e| format!("commit OCR history: {e}"))?;
    let data = conn
        .serialize(DatabaseName::Main)
        .map_err(|e| format!("serialize OCR history sqlite: {e}"))?;
    let encoded = encode_ocr_history_store(&data)?;
    let temp_path = path.with_extension("sqlite.write.tmp");
    let _ = fs::remove_file(&temp_path);
    fs::write(&temp_path, encoded).map_err(|e| format!("write OCR history sqlite: {e}"))?;
    replace_ocr_history_file_atomically(&temp_path, path)
}

#[cfg(windows)]
pub(super) fn replace_ocr_history_file_atomically(
    temp_path: &Path,
    final_path: &Path,
) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let temp_wide: Vec<u16> = temp_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let final_wide: Vec<u16> = final_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let ok = unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            final_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(format!(
            "replace OCR history sqlite: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
pub(super) fn replace_ocr_history_file_atomically(
    temp_path: &Path,
    final_path: &Path,
) -> Result<(), String> {
    fs::rename(temp_path, final_path).map_err(|e| format!("replace OCR history sqlite: {e}"))
}

pub(super) fn one_line_preview(text: &str, limit: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out = String::new();
    for (idx, ch) in compact.chars().enumerate() {
        if idx >= limit {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

pub(super) fn remove_spaces(text: &str) -> String {
    text.chars()
        .filter(|ch| !matches!(ch, ' ' | '\t' | '\u{3000}'))
        .collect()
}

fn needs_word_separator(left: &str, right: &str) -> bool {
    let Some(a) = left.chars().next_back() else {
        return false;
    };
    let Some(b) = right.chars().next() else {
        return false;
    };
    (a.is_ascii_alphanumeric() && b.is_ascii_alphanumeric())
        || (a.is_alphabetic() && b.is_alphabetic() && (a.is_ascii() || b.is_ascii()))
}

fn join_wrapped_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if needs_word_separator(&out, line) {
            out.push(' ');
        }
        out.push_str(line);
    }
    out
}

pub(super) fn remove_line_breaks_smart(text: &str) -> String {
    join_wrapped_lines(text.lines())
}

pub(super) fn keep_paragraphs_smart(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut paragraphs = Vec::new();
    let mut current = Vec::new();
    for line in normalized.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paragraphs.push(join_wrapped_lines(current.drain(..)));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paragraphs.push(join_wrapped_lines(current));
    }
    paragraphs.join("\n\n")
}

fn line_bounds(line: &OcrReviewLine) -> Option<(f32, f32, f32, f32)> {
    let points = line.bounds.as_ref()?;
    Some((
        points
            .iter()
            .map(|point| point[0])
            .fold(f32::INFINITY, f32::min),
        points
            .iter()
            .map(|point| point[1])
            .fold(f32::INFINITY, f32::min),
        points
            .iter()
            .map(|point| point[0])
            .fold(f32::NEG_INFINITY, f32::max),
        points
            .iter()
            .map(|point| point[1])
            .fold(f32::NEG_INFINITY, f32::max),
    ))
}

pub(super) fn table_mode_with_positions(text: &str, lines: &[OcrReviewLine]) -> String {
    let mut positioned = lines
        .iter()
        .filter_map(|line| line_bounds(line).map(|bounds| (line, bounds)))
        .collect::<Vec<_>>();
    if positioned.len() < 2 {
        return text.to_string();
    }
    positioned.sort_by(|a, b| {
        let ay = (a.1 .1 + a.1 .3) * 0.5;
        let by = (b.1 .1 + b.1 .3) * 0.5;
        ay.total_cmp(&by).then_with(|| a.1 .0.total_cmp(&b.1 .0))
    });
    let mut rows: Vec<Vec<(&OcrReviewLine, (f32, f32, f32, f32))>> = Vec::new();
    for item in positioned {
        let center = (item.1 .1 + item.1 .3) * 0.5;
        let height = (item.1 .3 - item.1 .1).max(1.0);
        let same_row = rows.last().is_some_and(|row| {
            let first = row[0].1;
            let row_center = (first.1 + first.3) * 0.5;
            let row_height = (first.3 - first.1).max(1.0);
            (center - row_center).abs() <= height.min(row_height) * 0.6
        });
        if same_row {
            rows.last_mut().expect("row exists").push(item);
        } else {
            rows.push(vec![item]);
        }
    }
    for row in &mut rows {
        row.sort_by(|a, b| a.1 .0.total_cmp(&b.1 .0));
    }
    if !rows.iter().any(|row| row.len() >= 2) {
        return text.to_string();
    }
    rows.into_iter()
        .map(|row| {
            row.into_iter()
                .map(|(line, _)| line.text.trim())
                .filter(|cell| !cell.is_empty())
                .collect::<Vec<_>>()
                .join("\t")
        })
        .filter(|row| !row.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn normalize_spaces(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn table_to_markdown(table: &str) -> String {
    let rows = table
        .lines()
        .map(|line| line.split('\t').map(str::trim).collect::<Vec<_>>())
        .filter(|row| !row.is_empty() && row.iter().any(|cell| !cell.is_empty()))
        .collect::<Vec<_>>();
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    if columns == 0 {
        return String::new();
    }
    let render = |row: &[&str]| {
        let cells = (0..columns)
            .map(|index| row.get(index).copied().unwrap_or("").replace('|', "\\|"))
            .collect::<Vec<_>>();
        format!("| {} |", cells.join(" | "))
    };
    let mut out = vec![render(&rows[0])];
    out.push(format!("| {} |", vec!["---"; columns].join(" | ")));
    out.extend(rows.iter().skip(1).map(|row| render(row)));
    out.join("\n")
}

pub(super) fn target_hwnd_from_args() -> Option<isize> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--target-hwnd" {
            return args.next().and_then(|value| value.parse::<isize>().ok());
        }
    }
    None
}

pub(super) fn image_path_from_args() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if matches!(arg.as_str(), "--image" | "--file" | "--screenshot") {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

pub(super) fn translate_after_ocr_from_args() -> bool {
    std::env::args().skip(1).any(|arg| arg == "--translate")
}

pub(super) fn manual_region_from_args() -> bool {
    std::env::args()
        .skip(1)
        .any(|arg| matches!(arg.as_str(), "--manual-region" | "--region"))
}

pub(super) fn delete_source_after_import_from_args() -> bool {
    std::env::args()
        .skip(1)
        .any(|arg| arg == "--delete-source-after-import")
}

pub(super) fn ocr_window_request_from_args() -> OcrWindowRequest {
    let target_hwnd = target_hwnd_from_args();
    let image = image_path_from_args();
    let manual_region = manual_region_from_args() || (target_hwnd.is_none() && image.is_none());
    OcrWindowRequest {
        created_ms: now_millis(),
        target_hwnd: target_hwnd.unwrap_or(0),
        manual_region,
        translate: translate_after_ocr_from_args(),
        image,
        delete_source_after_import: delete_source_after_import_from_args(),
    }
}
