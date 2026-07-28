//! 优先使用「微软雅黑」(msyh.ttc)，并注册常见 CJK fallback，保证 egui 正确显示多语言输入法名称。

use egui::{FontData, FontDefinitions, FontFamily};
use fontdb::{Database, Family, Query, Style, Weight};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 与 Windows 字体目录中的微软雅黑常规体文件名一致
const MSYH_WINDOWS: &str = r"C:\Windows\Fonts\msyh.ttc";

const HARMONY_FONT_KEY: &str = "harmonyos_sans_sc";
const CJK_FONT_KEY: &str = "microsoft_yahei";
pub const CANDIDATE_PREVIEW_FONT_FAMILY: &str = "kaixin_candidate_preview";
const CANDIDATE_PREVIEW_FONT_FAMILY_PREFIX: &str = "kaixin_candidate_preview_";
pub const SIMSUN_FONT_FAMILY: &str = "kaixin_clipboard_simsun";

struct LoadedFont {
    key: &'static str,
    bytes: Vec<u8>,
    label: String,
    path: Option<PathBuf>,
}

#[allow(dead_code)]
pub struct InstalledCjkFonts {
    pub label: String,
    pub candidate_preview_keys: BTreeSet<String>,
    pub requested_preview_loaded: bool,
}

/// 注册字体：优先微软雅黑，失败再查系统字体库。
#[allow(dead_code)]
pub fn install_cjk_fonts(ctx: &egui::Context) -> Option<String> {
    install_cjk_fonts_with_candidate_preview(ctx, None).map(|installed| installed.label)
}

#[allow(dead_code)]
pub fn install_cjk_fonts_with_report(ctx: &egui::Context) -> Option<InstalledCjkFonts> {
    install_cjk_fonts_with_candidate_preview(ctx, None)
}

#[allow(dead_code)]
pub fn candidate_preview_font_family(
    family: &str,
    registered_preview_keys: &BTreeSet<String>,
) -> Option<FontFamily> {
    let key = preview_font_key_for_family(family)?;
    if !registered_preview_keys.contains(key) {
        return None;
    }
    Some(FontFamily::Name(
        candidate_preview_font_family_name(key).into(),
    ))
}

fn install_cjk_fonts_with_candidate_preview(
    ctx: &egui::Context,
    preview_family: Option<&str>,
) -> Option<InstalledCjkFonts> {
    let primary = load_default_cjk_font_first()?;
    let mut fonts = FontDefinitions::default();
    let mut registered_keys = Vec::new();
    fonts
        .font_data
        .insert(primary.key.to_owned(), FontData::from_owned(primary.bytes));
    registered_keys.push(primary.key.to_owned());

    let mut loaded_paths = BTreeSet::new();
    if let Some(path) = primary.path.as_ref() {
        loaded_paths.insert(path.clone());
    }
    for (key, path) in cjk_fallback_font_paths() {
        let path = Path::new(path);
        if !loaded_paths.insert(path.to_path_buf()) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(path) {
            fonts
                .font_data
                .insert((*key).to_string(), FontData::from_owned(bytes));
            registered_keys.push((*key).to_string());
        }
    }

    register_candidate_preview_families(&mut fonts, &registered_keys);

    let preview_loaded = preview_family
        .map(str::trim)
        .filter(|family| !family.is_empty())
        .and_then(preview_font_key_for_family)
        .filter(|preview_key| registered_keys.iter().any(|key| key == *preview_key))
        .map(|preview_key| {
            register_font_family(
                &mut fonts,
                CANDIDATE_PREVIEW_FONT_FAMILY.to_owned(),
                preview_key,
                &registered_keys,
            );
            true
        })
        .unwrap_or(false);

    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        let family_fonts = fonts.families.entry(family).or_default();
        for (index, key) in registered_keys.iter().enumerate() {
            family_fonts.insert(index, key.to_owned());
        }
    }
    if registered_keys.iter().any(|key| key == "simsun") {
        let simsun_family = fonts
            .families
            .entry(FontFamily::Name(SIMSUN_FONT_FAMILY.into()))
            .or_default();
        simsun_family.push("simsun".to_owned());
        simsun_family.extend(
            registered_keys
                .iter()
                .filter(|key| *key != "simsun")
                .cloned(),
        );
    }
    ctx.set_fonts(fonts);
    Some(InstalledCjkFonts {
        label: primary.label,
        candidate_preview_keys: registered_keys.iter().cloned().collect(),
        requested_preview_loaded: preview_loaded,
    })
}

fn register_candidate_preview_families(fonts: &mut FontDefinitions, registered_keys: &[String]) {
    for key in registered_keys {
        register_font_family(
            fonts,
            candidate_preview_font_family_name(key),
            key,
            registered_keys,
        );
    }
}

fn register_font_family(
    fonts: &mut FontDefinitions,
    family_name: String,
    primary_key: &str,
    registered_keys: &[String],
) {
    let family_fonts = fonts
        .families
        .entry(FontFamily::Name(family_name.into()))
        .or_default();
    family_fonts.clear();
    family_fonts.push(primary_key.to_owned());
    family_fonts.extend(
        registered_keys
            .iter()
            .filter(|key| key.as_str() != primary_key)
            .cloned(),
    );
}

fn candidate_preview_font_family_name(key: &str) -> String {
    format!("{CANDIDATE_PREVIEW_FONT_FAMILY_PREFIX}{key}")
}

#[allow(dead_code)]
pub fn installed_chinese_font_families() -> Vec<String> {
    let mut db = Database::new();
    db.load_system_fonts();

    let mut names = BTreeSet::new();
    if bundled_harmony_sans_path().is_some() {
        names.insert("HarmonyOS Sans SC".to_owned());
    }
    for face in db.faces() {
        for (family, _) in &face.families {
            let family = family.trim();
            if family.is_empty() {
                continue;
            }
            if is_likely_chinese_font_family(family) {
                names.insert(family.to_owned());
            }
        }
    }

    if cfg!(windows) {
        for family in [
            "Microsoft YaHei UI",
            "Microsoft YaHei",
            "SimSun",
            "NSimSun",
            "SimHei",
            "KaiTi",
            "FangSong",
            "DengXian",
            "Microsoft JhengHei UI",
            "Microsoft JhengHei",
            "MingLiU",
            "PMingLiU",
        ] {
            let query = Query {
                families: &[Family::Name(family)],
                weight: Weight::NORMAL,
                style: Style::Normal,
                ..Default::default()
            };
            if db.query(&query).is_some() {
                names.insert(family.to_owned());
            }
        }
    }

    names.into_iter().collect()
}

fn preview_font_key_for_family(family: &str) -> Option<&'static str> {
    match family.trim().to_ascii_lowercase().as_str() {
        "harmonyos sans" | "harmonyos sans sc" | "harmony os sans" | "harmony os sans sc" => {
            Some(HARMONY_FONT_KEY)
        }
        "microsoft yahei" | "microsoft yahei ui" => Some(CJK_FONT_KEY),
        "simsun" | "nsimsun" => Some("simsun"),
        "microsoft jhenghei" | "microsoft jhenghei ui" => Some("microsoft_jhenghei"),
        "yu gothic" | "yu gothic ui" => Some("yu_gothic_ui"),
        "meiryo" => Some("meiryo"),
        "malgun gothic" => Some("malgun_gothic"),
        "kaiti" | "楷体" => Some("kaiti"),
        "fangsong" | "仿宋" => Some("fangsong"),
        _ => None,
    }
}

fn bundled_harmony_sans_path() -> Option<PathBuf> {
    const RELATIVE_CANDIDATES: &[&str] = &[
        r"HarmonyOS Sans\HarmonyOS_Sans_SC.ttf",
        r"font1\HarmonyOS_Sans_SC.ttf",
        r"font1\HarmonyOS Sans\HarmonyOS_Sans_SC.ttf",
    ];
    for root in font_search_roots() {
        for rel in RELATIVE_CANDIDATES {
            let path = root.join(rel);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn font_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(dir) = std::env::current_dir() {
        push_root_and_parents(&mut roots, dir, 4);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            push_root_and_parents(&mut roots, dir.to_path_buf(), 4);
        }
    }
    roots
}

fn push_root_and_parents(roots: &mut Vec<PathBuf>, mut root: PathBuf, max_parents: usize) {
    for _ in 0..=max_parents {
        if !roots.iter().any(|existing| existing == &root) {
            roots.push(root.clone());
        }
        let Some(parent) = root.parent() else {
            break;
        };
        root = parent.to_path_buf();
    }
}

fn load_default_cjk_font_first() -> Option<LoadedFont> {
    if cfg!(windows) {
        if let Ok(bytes) = std::fs::read(MSYH_WINDOWS) {
            return Some(LoadedFont {
                key: CJK_FONT_KEY,
                bytes,
                label: "微软雅黑 (Microsoft YaHei, msyh.ttc)".to_owned(),
                path: Some(PathBuf::from(MSYH_WINDOWS)),
            });
        }
        let alt = r"C:\Windows\Fonts\msyhbd.ttc";
        if let Ok(bytes) = std::fs::read(alt) {
            return Some(LoadedFont {
                key: CJK_FONT_KEY,
                bytes,
                label: "微软雅黑 Bold (msyhbd.ttc)".to_owned(),
                path: Some(PathBuf::from(alt)),
            });
        }
    }

    let mut db = Database::new();
    db.load_system_fonts();

    for name in ["Microsoft YaHei", "Microsoft YaHei UI"] {
        let query = Query {
            families: &[Family::Name(name)],
            weight: Weight::NORMAL,
            style: Style::Normal,
            ..Default::default()
        };
        if let Some(id) = db.query(&query) {
            if let Some(face) = db.face(id) {
                if let Some(bytes) = read_face_source(&face.source) {
                    return Some(LoadedFont {
                        key: CJK_FONT_KEY,
                        bytes,
                        label: format!("{name}（系统字体）"),
                        path: None,
                    });
                }
            }
        }
    }

    if let Some(path) = bundled_harmony_sans_path() {
        if let Ok(bytes) = std::fs::read(&path) {
            return Some(LoadedFont {
                key: HARMONY_FONT_KEY,
                bytes,
                label: format!("HarmonyOS Sans SC ({})", path.display()),
                path: Some(path),
            });
        }
    }

    let fallback_names = [
        "SimHei",
        "NSimSun",
        "SimSun",
        "Noto Sans CJK SC",
        "Source Han Sans SC",
        "PingFang SC",
        "Hiragino Sans GB",
        "WenQuanYi Micro Hei",
    ];
    for name in fallback_names {
        let query = Query {
            families: &[Family::Name(name)],
            weight: Weight::NORMAL,
            style: Style::Normal,
            ..Default::default()
        };
        if let Some(id) = db.query(&query) {
            if let Some(face) = db.face(id) {
                if let Some(bytes) = read_face_source(&face.source) {
                    return Some(LoadedFont {
                        key: CJK_FONT_KEY,
                        bytes,
                        label: format!("{name}（回退）"),
                        path: None,
                    });
                }
            }
        }
    }

    let paths: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/System/Library/Fonts/PingFang.ttc",
            "/System/Library/Fonts/STHeiti Light.ttc",
        ]
    } else if cfg!(unix) {
        &[
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        ]
    } else {
        &[]
    };
    for p in paths {
        if let Ok(bytes) = std::fs::read(p) {
            return Some(LoadedFont {
                key: CJK_FONT_KEY,
                bytes,
                label: p.to_string(),
                path: Some(PathBuf::from(p)),
            });
        }
    }
    None
}

fn cjk_fallback_font_paths() -> &'static [(&'static str, &'static str)] {
    if cfg!(windows) {
        &[
            (CJK_FONT_KEY, MSYH_WINDOWS),
            ("microsoft_jhenghei", r"C:\Windows\Fonts\msjh.ttc"),
            ("yu_gothic_ui", r"C:\Windows\Fonts\YuGothR.ttc"),
            ("meiryo", r"C:\Windows\Fonts\meiryo.ttc"),
            ("malgun_gothic", r"C:\Windows\Fonts\malgun.ttf"),
            ("kaiti", r"C:\Windows\Fonts\simkai.ttf"),
            ("fangsong", r"C:\Windows\Fonts\simfang.ttf"),
            ("simsun", r"C:\Windows\Fonts\simsun.ttc"),
            ("segui_symbol", r"C:\Windows\Fonts\seguisym.ttf"),
            ("segui_emoji", r"C:\Windows\Fonts\seguiemj.ttf"),
        ]
    } else if cfg!(target_os = "macos") {
        &[
            ("pingfang", "/System/Library/Fonts/PingFang.ttc"),
            (
                "hiragino",
                "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
            ),
            ("apple_symbols", "/System/Library/Fonts/Apple Symbols.ttf"),
        ]
    } else if cfg!(unix) {
        &[
            (
                "noto_sans_cjk",
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            ),
            (
                "noto_color_emoji",
                "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf",
            ),
        ]
    } else {
        &[]
    }
}

#[allow(dead_code)]
fn is_likely_chinese_font_family(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.chars().any(is_cjk_char)
        || [
            "yahei",
            "jhenghei",
            "simsun",
            "nsimsun",
            "simhei",
            "kaiti",
            "fangsong",
            "dengxian",
            "mingliu",
            "pmingliu",
            "source han",
            "noto sans cjk",
            "noto serif cjk",
            "sarasa",
            "lxgw",
            "wqy",
            "wenquanyi",
            "pingfang",
            "hiragino sans gb",
            "songti",
            "heiti",
            "kaiti",
            "fangsong",
            "harmonyos",
            "harmony os",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
}

#[allow(dead_code)]
fn is_cjk_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2A6DF}'
            | '\u{2A700}'..='\u{2B73F}'
            | '\u{2B740}'..='\u{2B81F}'
            | '\u{2B820}'..='\u{2CEAF}'
            | '\u{2CEB0}'..='\u{2EBEF}'
            | '\u{30000}'..='\u{3134F}'
    )
}

fn read_face_source(source: &fontdb::Source) -> Option<Vec<u8>> {
    match source {
        fontdb::Source::File(path) => std::fs::read(path).ok(),
        fontdb::Source::Binary(bin) => {
            let slice: &[u8] = AsRef::<[u8]>::as_ref(&**bin);
            Some(slice.to_vec())
        }
        fontdb::Source::SharedFile(path, _) => std::fs::read(path).ok(),
    }
}
