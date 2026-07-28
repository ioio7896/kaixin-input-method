use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::OnceLock;

use rusqlite::{Connection, DatabaseName};

const S2T_CHARS_SQLITE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/s2t_chars.sqlite"));

fn sqlite_owned_data_from_bytes(bytes: &[u8]) -> rusqlite::serialize::OwnedData {
    let ptr = unsafe { rusqlite::ffi::sqlite3_malloc64(bytes.len() as u64) }.cast::<u8>();
    let ptr = NonNull::new(ptr).expect("allocate simplified-traditional sqlite store");
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr.as_ptr(), bytes.len());
        rusqlite::serialize::OwnedData::from_raw_nonnull(ptr, bytes.len())
    }
}

fn load_s2t_map() -> rusqlite::Result<HashMap<char, String>> {
    if S2T_CHARS_SQLITE.is_empty() {
        return Ok(HashMap::new());
    }
    let mut conn = Connection::open_in_memory()?;
    let data = sqlite_owned_data_from_bytes(S2T_CHARS_SQLITE);
    conn.deserialize(DatabaseName::Main, data, false)?;
    let mut stmt = conn.prepare(
        "SELECT simplified, traditional
         FROM s2t_chars
         ORDER BY sort_order",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut map = HashMap::new();
    for row in rows {
        let (src, dst) = row?;
        let Some(ch) = src.chars().next() else {
            continue;
        };
        if src.chars().count() == 1 && !dst.is_empty() {
            map.insert(ch, dst);
        }
    }
    Ok(map)
}

fn s2t_map() -> &'static HashMap<char, String> {
    static MAP: OnceLock<HashMap<char, String>> = OnceLock::new();
    MAP.get_or_init(|| load_s2t_map().unwrap_or_default())
}

pub fn to_traditional(text: &str) -> String {
    let map = s2t_map();
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    for ch in text.chars() {
        if let Some(converted) = map.get(&ch) {
            out.push_str(converted);
            changed = true;
        } else {
            out.push(ch);
        }
    }
    if changed {
        out
    } else {
        text.to_string()
    }
}
