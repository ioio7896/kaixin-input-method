//! Read optional lexicon switches from the user INI.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

fn user_config_ini_path() -> Option<PathBuf> {
    crate::app_paths::config_ini_path()
}

fn extract_lexicon_section_fingerprint_source(text: &str) -> String {
    let mut in_lexicon = false;
    let mut out = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            let name = line[1..line.len() - 1].trim().to_ascii_lowercase();
            in_lexicon = name == "lexicon";
            continue;
        }
        if in_lexicon {
            out.push_str(raw);
            out.push('\n');
        }
    }
    out
}

fn fingerprint_str(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

struct LexiconIniPoll {
    last_path: Option<PathBuf>,
    last_mtime: Option<SystemTime>,
    last_fp: Option<u64>,
}

static LEXICON_RELOAD_PENDING: AtomicBool = AtomicBool::new(false);

/// Background-loaded lexicon, ready for atomic swap by the engine.
/// When the INI watcher detects a change, it spawns a load in a background
/// thread so the next lookup can swap in the new lexicon without blocking.
pub static PREPARED_LEXICON: std::sync::Mutex<Option<crate::thuocl::AbbrevLexicon>> =
    std::sync::Mutex::new(None);

/// If a background thread has finished loading a new lexicon, take it.
/// Returns `None` if no prepared lexicon is available.
pub fn take_prepared_lexicon() -> Option<crate::thuocl::AbbrevLexicon> {
    PREPARED_LEXICON.lock().ok()?.take()
}

fn current_lexicon_fingerprint(state: &mut LexiconIniPoll) -> Option<u64> {
    let path = user_config_ini_path()?;
    let meta = std::fs::metadata(&path).ok();
    let modified = meta.as_ref().and_then(|m| m.modified().ok());
    if state.last_path.as_ref() == Some(&path) && modified.is_some() && modified == state.last_mtime
    {
        return state.last_fp;
    }

    let text = std::fs::read_to_string(&path).ok()?;
    let src = extract_lexicon_section_fingerprint_source(&text);
    let fp = fingerprint_str(&src);
    state.last_path = Some(path);
    state.last_mtime = modified;
    Some(fp)
}

fn lexicon_ini_watcher_loop() {
    let mut state = LexiconIniPoll {
        last_path: None,
        last_mtime: None,
        last_fp: None,
    };
    loop {
        let current = current_lexicon_fingerprint(&mut state);
        match (state.last_fp, current) {
            (None, None) => {}
            (None, Some(fp)) => state.last_fp = Some(fp),
            (Some(prev), Some(fp)) => {
                if prev != fp {
                    LEXICON_RELOAD_PENDING.store(true, Ordering::Release);
                }
                state.last_fp = Some(fp);
            }
            (Some(_), None) => {
                state.last_path = None;
                state.last_mtime = None;
                state.last_fp = None;
                LEXICON_RELOAD_PENDING.store(true, Ordering::Release);
            }
        }
        std::thread::sleep(Duration::from_millis(800));
    }
}

fn ensure_lexicon_ini_watcher_started() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("srf-lexicon-ini-watch".to_string())
            .spawn(lexicon_ini_watcher_loop);
    });
}

pub fn take_phrase_lexicon_reload_flag() -> bool {
    ensure_lexicon_ini_watcher_started();
    LEXICON_RELOAD_PENDING.swap(false, Ordering::AcqRel)
}

pub fn request_phrase_lexicon_reload() {
    ensure_lexicon_ini_watcher_started();
    LEXICON_RELOAD_PENDING.store(true, Ordering::Release);
}

pub fn should_reload_phrase_lexicon_from_ini() -> bool {
    take_phrase_lexicon_reload_flag()
}

fn parse_lexicon_section_bool(text: &str) -> HashMap<String, bool> {
    let mut out = HashMap::new();
    let mut in_lexicon = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') && line.len() >= 2 {
            let name = line[1..line.len() - 1].trim().to_ascii_lowercase();
            in_lexicon = name == "lexicon";
            continue;
        }
        if !in_lexicon {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim().to_ascii_lowercase();
            let on = matches!(val.as_str(), "1" | "true" | "yes" | "on");
            let off = matches!(val.as_str(), "0" | "false" | "no" | "off");
            if on {
                out.insert(key, true);
            } else if off {
                out.insert(key, false);
            }
        }
    }
    out
}

pub fn thuocl_basename_tag(file_name: &str) -> Option<String> {
    let lower = file_name.to_ascii_lowercase();
    let base = lower.strip_suffix(".txt")?;
    if let Some(rest) = base.strip_prefix("thuocl_") {
        if !rest.is_empty() {
            return Some(rest.to_string());
        }
    }
    const MARKER: &str = "__thuocl_";
    if let Some(idx) = base.find(MARKER) {
        let tag = &base[idx + MARKER.len()..];
        if !tag.is_empty() {
            return Some(tag.to_string());
        }
    }
    None
}

pub fn optional_lexicon_path_tag(path: &Path) -> Option<String> {
    // Only Ext-layer files are user-switchable. In particular, a legacy
    // THUOCL-style filename under zh must not make the curated main lexicon
    // optional: zh is always loaded, while zh-ext/ext can be disabled.
    if crate::thuocl::LexiconLayer::from_path(path) != crate::thuocl::LexiconLayer::Ext {
        return None;
    }
    let file_name = path.file_name().and_then(|name| name.to_str())?;
    if let Some(tag) = thuocl_basename_tag(file_name) {
        return Some(tag);
    }
    let lower = file_name.to_ascii_lowercase();
    let base = lower.strip_suffix(".txt")?;
    (!base.is_empty()).then_some(base.to_string())
}

fn should_skip_lexicon_subdir(name: &str) -> bool {
    matches!(name, "lua" | "opencc" | "en_dicts" | ".git")
}

pub fn discover_optional_lexicon_tags_in_lexicon_dir(lex_dir: &Path) -> Vec<(String, String)> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    let _ = walk_optional_lexicon_files(lex_dir, &mut map);
    map.into_iter().collect()
}

fn walk_optional_lexicon_files(dir: &Path, map: &mut BTreeMap<String, String>) -> io::Result<()> {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if should_skip_lexicon_subdir(name) {
                continue;
            }
            walk_optional_lexicon_files(&path, map)?;
            continue;
        }
        let Some(tag) = optional_lexicon_path_tag(&path) else {
            continue;
        };
        let fname = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        map.entry(tag).or_insert(fname);
    }
    Ok(())
}

fn lexicon_toggle_map() -> Option<HashMap<String, bool>> {
    let path = user_config_ini_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(parse_lexicon_section_bool(&text))
}

pub fn default_optional_lexicon_tag_enabled(_tag: &str) -> bool {
    true
}

pub fn is_optional_lexicon_tag_enabled(tag: &str) -> bool {
    let Some(map) = lexicon_toggle_map() else {
        return default_optional_lexicon_tag_enabled(tag);
    };
    let key = format!("lexicon_{}", tag.to_ascii_lowercase());
    map.get(&key)
        .copied()
        .unwrap_or_else(|| default_optional_lexicon_tag_enabled(tag))
}

pub fn is_thuocl_tag_enabled(tag: &str) -> bool {
    is_optional_lexicon_tag_enabled(tag)
}

pub fn has_custom_optional_lexicon_prefs() -> bool {
    lexicon_toggle_map().is_some_and(|map| {
        map.into_iter().any(|(key, enabled)| {
            let Some(tag) = key.strip_prefix("lexicon_") else {
                return false;
            };
            enabled != default_optional_lexicon_tag_enabled(tag)
        })
    })
}

pub fn has_disabled_optional_lexicon_tags() -> bool {
    has_custom_optional_lexicon_prefs()
}

pub fn filter_thuocl_paths_by_prefs(paths: &mut Vec<PathBuf>) {
    filter_optional_lexicon_paths_by_prefs(paths);
}

pub fn filter_optional_lexicon_paths_by_default(paths: &mut Vec<PathBuf>) {
    paths.retain(|path| optional_lexicon_path_enabled(path, None));
}

pub fn filter_optional_lexicon_paths_by_prefs(paths: &mut Vec<PathBuf>) {
    let map = lexicon_toggle_map();
    paths.retain(|path| optional_lexicon_path_enabled(path, map.as_ref()));
}

fn optional_lexicon_path_enabled(path: &Path, map: Option<&HashMap<String, bool>>) -> bool {
    let Some(tag) = optional_lexicon_path_tag(path) else {
        // Main zh/Core/Base/Large lexicons are mandatory and ignore toggle
        // keys, including stale keys left by an older settings version.
        return true;
    };
    let key = format!("lexicon_{}", tag.to_ascii_lowercase());
    map.and_then(|prefs| prefs.get(&key).copied())
        .unwrap_or_else(|| default_optional_lexicon_tag_enabled(&tag))
}
