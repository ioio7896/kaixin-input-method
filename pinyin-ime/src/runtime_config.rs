//! Unified immutable runtime configuration snapshot.
//!
//! A background watcher is the only code that touches `kaixin.ini`; lookup
//! threads only clone the current in-memory `Arc` under a short read lock.

use crate::correction_prefs::CorrectionPrefs;
use crate::fuzzy_prefs::FuzzyPairs;
use crate::rerank_prefs::RerankPrefs;
use crate::user_hotword_prefs::UserHotwordPrefs;
use std::sync::{Arc, OnceLock, RwLock};
use std::time::{Duration, SystemTime};

const CONFIG_POLL_INTERVAL: Duration = Duration::from_millis(800);

#[derive(Clone, Debug)]
pub struct RuntimeConfigSnapshot {
    pub candidate_page_size: usize,
    pub effective_candidate_page_size: usize,
    pub correction: CorrectionPrefs,
    pub fuzzy: FuzzyPairs,
    pub rerank: RerankPrefs,
    pub user_hotword: UserHotwordPrefs,
    pub shortcuts: Arc<[(String, Vec<String>)]>,
    pub(crate) engine_tuning: crate::core::EngineTuning,
    pub generation: u64,
}

impl RuntimeConfigSnapshot {
    fn parse(text: &str, generation: u64) -> Self {
        let candidate_page = crate::candidate_prefs::parse_candidate_page_prefs(text);
        Self {
            candidate_page_size: candidate_page.page_size,
            effective_candidate_page_size: if candidate_page.horizontal {
                candidate_page
                    .page_size
                    .min(candidate_page.horizontal_count)
            } else {
                candidate_page.page_size
            },
            correction: crate::correction_prefs::parse_correction_section(text),
            fuzzy: crate::fuzzy_prefs::parse_fuzzy_section(text),
            rerank: crate::rerank_prefs::parse_rank_section(text),
            user_hotword: crate::user_hotword_prefs::parse_engine_section(text),
            shortcuts: Arc::from(crate::custom_shortcuts::parse_shortcuts_section(text)),
            engine_tuning: crate::core::parse_engine_tuning_ini(text),
            generation,
        }
    }
}

impl Default for RuntimeConfigSnapshot {
    fn default() -> Self {
        Self::parse("", 0)
    }
}

struct ConfigStore {
    current: RwLock<Arc<RuntimeConfigSnapshot>>,
}

static STORE: OnceLock<ConfigStore> = OnceLock::new();

fn store() -> &'static ConfigStore {
    STORE.get_or_init(|| {
        let initial =
            load_snapshot(1).unwrap_or_else(|| Arc::new(RuntimeConfigSnapshot::default()));
        let store = ConfigStore {
            current: RwLock::new(initial),
        };
        let _ = std::thread::Builder::new()
            .name("kaixin-config-watch".to_string())
            .spawn(config_watch_loop);
        store
    })
}

pub fn snapshot() -> Arc<RuntimeConfigSnapshot> {
    store()
        .current
        .read()
        .map(|value| Arc::clone(&value))
        .unwrap_or_else(|_| Arc::new(RuntimeConfigSnapshot::default()))
}

fn file_stamp() -> Option<(SystemTime, u64)> {
    let path = crate::app_paths::config_ini_path()?;
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

fn load_snapshot(generation: u64) -> Option<Arc<RuntimeConfigSnapshot>> {
    let path = crate::app_paths::config_ini_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(Arc::new(RuntimeConfigSnapshot::parse(&text, generation)))
}

fn config_watch_loop() {
    let mut stamp = file_stamp();
    loop {
        std::thread::sleep(CONFIG_POLL_INTERVAL);
        let next_stamp = file_stamp();
        if next_stamp == stamp {
            continue;
        }
        let generation = snapshot().generation.wrapping_add(1);
        let next = if next_stamp.is_some() {
            load_snapshot(generation)
        } else {
            Some(Arc::new(RuntimeConfigSnapshot::parse("", generation)))
        };
        if let Some(next) = next {
            if let Ok(mut current) = store().current.write() {
                *current = next;
                stamp = next_stamp;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_runtime_sections_together() {
        let snapshot = RuntimeConfigSnapshot::parse(
            "[style]\ncandidate_page_size=5\n[correction]\nenabled=false\n[fuzzy]\nzh_z=false\n[rank]\nw_single_lm=.5\n[engine]\nuser_hotword_boost=strong\n[shortcuts]\nqq=测试\n",
            7,
        );
        assert_eq!(snapshot.candidate_page_size, 5);
        assert!(!snapshot.correction.enabled);
        assert!(!snapshot.fuzzy.zh_z);
        assert_eq!(snapshot.rerank.w_single_lm, 0.5);
        assert_eq!(snapshot.user_hotword.front_limit, 3);
        assert_eq!(snapshot.shortcuts.len(), 1);
        assert_eq!(snapshot.generation, 7);
    }
}
