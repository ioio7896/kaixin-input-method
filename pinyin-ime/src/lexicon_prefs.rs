//! Read optional lexicon switches from the user INI.

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::io;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

include!("retired_lexicons.generated.rs");
include!("category_lexicons.generated.rs");

pub fn is_retired_optional_lexicon_tag(tag: &str) -> bool {
    RETIRED_OPTIONAL_LEXICON_TAGS
        .iter()
        .any(|retired| retired.eq_ignore_ascii_case(tag))
        || tag == "药物2026"
}

pub fn is_retired_packaged_lexicon_path(path: &Path) -> bool {
    RETIRED_PACKAGED_LEXICON_FILES
        .iter()
        .any(|relative| path.ends_with(Path::new(relative)))
}

/// Retirement applies only to optional dictionaries, never the main lexicon.
pub fn is_retired_optional_lexicon_path(path: &Path) -> bool {
    optional_lexicon_path_tag(path).is_some_and(|tag| is_retired_optional_lexicon_tag(&tag))
}

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
    // Old installations may still have these correction tables under zh-ext.
    // Their placement must not turn mandatory pronunciation data into switches.
    if is_foundation_lexicon(file_name) {
        return None;
    }
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
        if is_retired_optional_lexicon_tag(&tag) {
            continue;
        }
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

pub fn default_optional_lexicon_tag_enabled(tag: &str) -> bool {
    !is_retired_optional_lexicon_tag(tag)
        && !tag.to_ascii_lowercase().starts_with("hangzhou_")
        && !tag.to_ascii_lowercase().starts_with("professional_")
}

fn is_foundation_lexicon(file_name: &str) -> bool {
    if let Some(tag) = thuocl_basename_tag(file_name) {
        if matches!(
            tag.as_str(),
            "kaixin_explicit" | "kaixin_polyphone" | "kaixin_pronunciation_aliases"
        ) {
            return true;
        }
    }
    matches!(
        file_name.to_ascii_lowercase().as_str(),
        "kaixin_explicit.txt" | "kaixin_polyphone.txt" | "kaixin_pronunciation_aliases.txt"
    )
}

pub struct OptionalLexiconInfo {
    pub tag: String,
    pub entries: usize,
    pub files: Vec<String>,
    pub incomplete: bool,
}

/// Count source records on demand, outside the per-frame settings rendering.
/// Counts are records (including alternate readings), not unique words.
pub fn discover_optional_lexicon_info(root: &Path) -> Vec<OptionalLexiconInfo> {
    fn visit(dir: &Path, items: &mut BTreeMap<String, OptionalLexiconInfo>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if !should_skip_lexicon_subdir(
                    path.file_name().and_then(|s| s.to_str()).unwrap_or(""),
                ) {
                    visit(&path, items);
                }
                continue;
            }
            let Some(tag) = optional_lexicon_path_tag(&path) else {
                continue;
            };
            if is_retired_optional_lexicon_tag(&tag) {
                continue;
            }
            let item = items
                .entry(tag.clone())
                .or_insert_with(|| OptionalLexiconInfo {
                    tag,
                    entries: 0,
                    files: Vec::new(),
                    incomplete: false,
                });
            item.files.push(path.display().to_string());
            match std::fs::File::open(&path) {
                Ok(file) => {
                    for line in io::BufReader::new(file).lines() {
                        match line {
                            Ok(line) => {
                                let line = line.trim_start_matches('\u{feff}').trim();
                                if !line.is_empty()
                                    && !line.starts_with('#')
                                    && !line.starts_with(';')
                                {
                                    item.entries += 1;
                                }
                            }
                            Err(_) => {
                                item.incomplete = true;
                                break;
                            }
                        }
                    }
                }
                Err(_) => item.incomplete = true,
            }
        }
    }
    let mut items = BTreeMap::new();
    visit(root, &mut items);
    items.into_values().collect()
}

pub fn optional_lexicon_group(tag: &str) -> &'static str {
    if let Some((_, group)) = category_lexicon_info(tag) {
        return group;
    }
    if tag.starts_with("professional_") {
        return "专业领域";
    }
    if tag.starts_with("hangzhou_") {
        return "地区词库 · 杭州";
    }
    match tag {
        "daily_communication" => "日常表达",
        "animals" | "people_names" => "名称与实体",
        "technology" => "科技与开发",
        "medicine" => "药物名称",
        "geography_admin" => "地区词库 · 全国与世界",
        _ => "其他扩展",
    }
}

pub fn optional_lexicon_description(tag: &str) -> &'static str {
    if category_lexicon_info(tag).is_some() {
        return "分类补充词库；保留基础词频与实际读音，默认开启，可独立关闭。";
    }
    if tag.starts_with("professional_") {
        return "精选专业基础术语小词库；默认关闭，按需勾选后保存生效。";
    }
    if tag.starts_with("hangzhou_") {
        return "杭州地名、交通、公共设施与本地生活；建议按需开启。";
    }
    match tag {
        "daily_communication" => "精选真实聊天与办公表达；不以通用词凑足条数。",
        "animals" => "精选 500 条常见动物名称与相关用语。",
        "people_names" => "姓氏、人物姓名及相应读音。",
        "technology" => "人工智能、互联网产品、编程框架与云服务。",
        "medicine" => "统一收录常见及补充药物通用名；可独立开关，与医疗术语分开。",
        "geography_admin" => "行政区划、国家及主要城市名称。",
        _ => "按实际安装的词库文件加载；可通过名称或文件标识搜索。",
    }
}

pub fn legacy_optional_lexicon_tags(tag: &str) -> &'static [&'static str] {
    match tag {
        "technology" => &[
            "ai_and_machine_learning",
            "internet_products",
            "programming_frameworks",
            "software_and_cloud",
        ],
        "daily_communication" => &["chat_common_phrases", "office_common_phrases"],
        "geography_admin" => &[
            "china_prefecture_level_admin_333",
            "county_admin_short_names_2024",
            "world_countries_major_cities",
        ],
        "hangzhou_local" => &[
            "hangzhou_admin",
            "hangzhou_business",
            "hangzhou_food_culture",
            "hangzhou_landmarks_life",
            "hangzhou_local_culture",
            "hangzhou_metro_stations_262",
            "hangzhou_new_places",
            "hangzhou_public_services",
            "hangzhou_transport",
        ],
        "people_names" => &["chinese_surnames", "name1"],
        "animals" => &["animal_common_5000"],
        "medicine" => &["yaowu", "药物2026"],
        _ => &[],
    }
}

fn optional_lexicon_tag_enabled_from_map(tag: &str, map: &HashMap<String, bool>) -> bool {
    if is_retired_optional_lexicon_tag(tag) {
        return false;
    }
    let key = format!("lexicon_{}", tag.to_ascii_lowercase());
    if let Some(enabled) = map.get(&key).copied() {
        return enabled;
    }
    let legacy_values = legacy_optional_lexicon_tags(tag)
        .iter()
        .filter_map(|legacy| map.get(&format!("lexicon_{legacy}")).copied())
        .collect::<Vec<_>>();
    if legacy_values.is_empty() {
        default_optional_lexicon_tag_enabled(tag)
    } else {
        legacy_values.into_iter().all(|enabled| enabled)
    }
}

pub fn is_optional_lexicon_tag_enabled(tag: &str) -> bool {
    let Some(map) = lexicon_toggle_map() else {
        return default_optional_lexicon_tag_enabled(tag);
    };
    optional_lexicon_tag_enabled_from_map(tag, &map)
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
            !is_retired_optional_lexicon_tag(tag)
                && enabled != default_optional_lexicon_tag_enabled(tag)
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
    map.map(|prefs| optional_lexicon_tag_enabled_from_map(&tag, prefs))
        .unwrap_or_else(|| default_optional_lexicon_tag_enabled(&tag))
}

#[cfg(test)]
mod retirement_tests {
    use super::*;

    #[test]
    fn stale_enabled_switches_cannot_restore_retired_sources() {
        let map = parse_lexicon_section_bool(
            "[lexicon]\nlexicon_animal=1\nlexicon_ai_and_machine_learning=1\n",
        );
        assert!(!optional_lexicon_path_enabled(
            Path::new("lexicon/zh-ext/THUOCL_animal.txt"),
            Some(&map),
        ));
        assert!(!optional_lexicon_tag_enabled_from_map(
            "ai_and_machine_learning",
            &map,
        ));
        assert!(optional_lexicon_path_enabled(
            Path::new("lexicon/zh/THUOCL_animal.txt"),
            Some(&map),
        ));
    }

    #[test]
    fn modern_switches_still_inherit_explicit_legacy_preferences() {
        let map = parse_lexicon_section_bool("[lexicon]\nlexicon_animal_common_5000=0\n");
        assert!(!optional_lexicon_tag_enabled_from_map("animals", &map));
        let map = parse_lexicon_section_bool(
            "[lexicon]\nlexicon_animal_common_5000=0\nlexicon_animals=1\n",
        );
        assert!(optional_lexicon_tag_enabled_from_map("animals", &map));
    }
}
