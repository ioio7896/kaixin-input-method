//! Runtime preferences for learned exact-input hotword promotion.

#[cfg(test)]
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserHotwordBoostLevel {
    Conservative,
    Standard,
    Strong,
    Aggressive,
}

#[derive(Clone, Copy, Debug)]
pub struct UserHotwordPrefs {
    pub level: UserHotwordBoostLevel,
    pub exact_bonus_scale: f64,
    pub signal_scale: f64,
    pub trust_floor: f64,
    pub front_limit: usize,
}

impl UserHotwordPrefs {
    fn from_level(level: UserHotwordBoostLevel) -> Self {
        match level {
            UserHotwordBoostLevel::Conservative => Self {
                level,
                exact_bonus_scale: 0.90,
                signal_scale: 0.85,
                trust_floor: 0.30,
                front_limit: 0,
            },
            UserHotwordBoostLevel::Standard => Self {
                level,
                exact_bonus_scale: 1.08,
                signal_scale: 1.20,
                trust_floor: 0.68,
                front_limit: 2,
            },
            UserHotwordBoostLevel::Strong => Self {
                level,
                exact_bonus_scale: 1.25,
                signal_scale: 1.45,
                trust_floor: 0.86,
                front_limit: 3,
            },
            UserHotwordBoostLevel::Aggressive => Self {
                level,
                exact_bonus_scale: 1.45,
                signal_scale: 1.75,
                trust_floor: 1.00,
                front_limit: 4,
            },
        }
    }
}

impl Default for UserHotwordPrefs {
    fn default() -> Self {
        Self::from_level(UserHotwordBoostLevel::Standard)
    }
}

#[cfg(test)]
static TEST_PREFS: OnceLock<Mutex<Option<UserHotwordPrefs>>> = OnceLock::new();

fn parse_level(value: &str) -> UserHotwordBoostLevel {
    match value.trim().to_ascii_lowercase().as_str() {
        "conservative" | "low" | "soft" => UserHotwordBoostLevel::Conservative,
        "strong" | "high" => UserHotwordBoostLevel::Strong,
        "aggressive" | "max" => UserHotwordBoostLevel::Aggressive,
        _ => UserHotwordBoostLevel::Standard,
    }
}

pub(crate) fn parse_engine_section(text: &str) -> UserHotwordPrefs {
    let mut level = UserHotwordBoostLevel::Standard;
    let mut in_engine = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            let name = line[1..line.len() - 1].trim().to_ascii_lowercase();
            in_engine = name == "engine";
            continue;
        }
        if !in_engine {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim()
            .eq_ignore_ascii_case(crate::config_schema::key::USER_HOTWORD_BOOST)
        {
            level = parse_level(v);
        }
    }
    UserHotwordPrefs::from_level(level)
}

#[cfg(test)]
fn test_prefs() -> Option<UserHotwordPrefs> {
    TEST_PREFS
        .get()
        .and_then(|prefs| prefs.lock().ok().and_then(|guard| *guard))
}

#[cfg(test)]
pub struct UserHotwordPrefsTestGuard;

#[cfg(test)]
impl Drop for UserHotwordPrefsTestGuard {
    fn drop(&mut self) {
        if let Some(prefs) = TEST_PREFS.get() {
            if let Ok(mut guard) = prefs.lock() {
                *guard = None;
            }
        }
    }
}

#[cfg(test)]
pub fn set_user_hotword_level_for_test(level: UserHotwordBoostLevel) -> UserHotwordPrefsTestGuard {
    let prefs = TEST_PREFS.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = prefs.lock() {
        *guard = Some(UserHotwordPrefs::from_level(level));
    }
    UserHotwordPrefsTestGuard
}

pub fn get_user_hotword_prefs() -> UserHotwordPrefs {
    #[cfg(test)]
    if let Some(prefs) = test_prefs() {
        return prefs;
    }
    crate::runtime_config::snapshot().user_hotword
}
