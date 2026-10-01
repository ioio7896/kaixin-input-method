//! Bounded, exact-only disk recall. No full lexicon graph is materialized.
use crate::thuocl::ThuoclEntry;
use rusqlite::{Connection, OpenFlags};
use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::Duration;

pub(crate) struct ColdLexicon {
    connection: Connection,
    cache: RefCell<lru::LruCache<String, Vec<ThuoclEntry>>>,
}

impl ColdLexicon {
    pub(crate) fn open(dir: &Path) -> Option<Self> {
        let connection = Connection::open_with_flags(
            dir.join("cold_lexicon.sqlite"),
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .ok()?;
        connection.busy_timeout(Duration::ZERO).ok()?;
        connection
            .execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-512; PRAGMA mmap_size=0;")
            .ok()?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .ok()?;
        (version == 1).then(|| Self {
            connection,
            cache: RefCell::new(lru::LruCache::new(
                NonZeroUsize::new(64).expect("nonzero cache size"),
            )),
        })
    }

    pub(crate) fn lookup(&self, key: &str) -> Vec<ThuoclEntry> {
        if let Some(entries) = self.cache.borrow_mut().get(key) {
            return entries.clone();
        }
        let Ok(mut statement) = self.connection.prepare_cached(
            "SELECT phrase, freq FROM entries WHERE reading=?1 ORDER BY ordinal LIMIT 16",
        ) else {
            return Vec::new();
        };
        let Ok(rows) = statement.query_map([key], |row| {
            Ok(ThuoclEntry {
                phrase: row.get(0)?,
                freq: row.get(1)?,
                code: None,
                pronunciation_kind: Default::default(),
            })
        }) else {
            return Vec::new();
        };
        let Ok(entries) = rows.collect::<rusqlite::Result<Vec<_>>>() else {
            return Vec::new();
        };
        self.cache
            .borrow_mut()
            .put(key.to_string(), entries.clone());
        entries
    }
}
