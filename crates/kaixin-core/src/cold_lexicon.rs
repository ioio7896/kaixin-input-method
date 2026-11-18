//! Bounded, exact-only disk recall. No full lexicon graph is materialized.
use crate::thuocl::ThuoclEntry;
use rusqlite::{params_from_iter, types::Value, Connection, OpenFlags};
use std::cell::RefCell;
use std::num::NonZeroUsize;
use std::path::Path;
use std::time::Duration;

pub(crate) struct ColdLexicon {
    connection: Connection,
    optional_tags: Vec<String>,
    cache: RefCell<lru::LruCache<(String, u64), Vec<ThuoclEntry>>>,
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
        if version != 2 {
            return None;
        }
        let optional_tags = {
            let mut statement = connection
                .prepare("SELECT tag FROM optional_tags ORDER BY tag")
                .ok()?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))
                .ok()?;
            rows.collect::<rusqlite::Result<Vec<_>>>().ok()?
        };
        Some(Self {
            connection,
            optional_tags,
            cache: RefCell::new(lru::LruCache::new(
                NonZeroUsize::new(64).expect("nonzero cache size"),
            )),
        })
    }

    pub(crate) fn lookup(&self, key: &str) -> Vec<ThuoclEntry> {
        let (prefs_fingerprint, enabled_tags) =
            crate::lexicon_prefs::optional_lexicon_selection(&self.optional_tags);
        let cache_key = (key.to_string(), prefs_fingerprint);
        if let Some(entries) = self.cache.borrow_mut().get(&cache_key) {
            return entries.clone();
        }
        let entries = query_cold_entries(&self.connection, key, &enabled_tags);
        self.cache.borrow_mut().put(cache_key, entries.clone());
        entries
    }
}

fn query_cold_entries(
    connection: &Connection,
    key: &str,
    enabled_tags: &[String],
) -> Vec<ThuoclEntry> {
    let optional_filter = if enabled_tags.is_empty() {
        String::from("source_tag=''")
    } else {
        format!(
            "source_tag='' OR source_tag IN ({})",
            (2..=enabled_tags.len() + 1)
                .map(|index| format!("?{index}"))
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    let sql = format!(
        "SELECT phrase, MAX(freq) FROM entries WHERE reading=?1 AND ({optional_filter}) \
         GROUP BY phrase ORDER BY MIN(ordinal), MAX(freq) DESC LIMIT 16"
    );
    let Ok(mut statement) = connection.prepare_cached(&sql) else {
        return Vec::new();
    };
    let mut params = Vec::with_capacity(enabled_tags.len() + 1);
    params.push(Value::Text(key.to_string()));
    params.extend(enabled_tags.iter().cloned().map(Value::Text));
    let Ok(rows) = statement.query_map(params_from_iter(params), |row| {
        Ok(ThuoclEntry {
            phrase: row.get(0)?,
            freq: row.get(1)?,
            code: None,
            pronunciation_kind: Default::default(),
        })
    }) else {
        return Vec::new();
    };
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_recall_filters_categories_and_deduplicates_phrases() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE entries (
                   reading TEXT NOT NULL, ordinal INTEGER NOT NULL, phrase TEXT NOT NULL,
                   freq INTEGER NOT NULL, source_tag TEXT NOT NULL
                 );
                 INSERT INTO entries VALUES
                   ('nihao', 0, '共有词', 100, ''),
                   ('nihao', 0, '共有词', 150, 'animals'),
                   ('nihao', 1, '动物词', 80, 'animals'),
                   ('nihao', 2, '游戏词', 70, 'games');",
            )
            .unwrap();

        let animals = query_cold_entries(&connection, "nihao", &["animals".to_string()]);
        assert_eq!(animals.len(), 2);
        assert_eq!(animals[0].phrase, "共有词");
        assert_eq!(animals[0].freq, 150);
        assert_eq!(animals[1].phrase, "动物词");

        let mandatory_only = query_cold_entries(&connection, "nihao", &[]);
        assert_eq!(mandatory_only.len(), 1);
        assert_eq!(mandatory_only[0].phrase, "共有词");
        assert_eq!(mandatory_only[0].freq, 100);
    }
}
