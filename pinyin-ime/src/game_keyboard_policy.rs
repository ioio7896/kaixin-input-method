//! Shared foreground policy for releasing global tool hotkeys in games.
use std::collections::BTreeMap;

pub fn wildcard_match(pattern: &str, value: &str) -> bool {
    let p = pattern.to_lowercase().into_bytes();
    let v = value.to_lowercase().into_bytes();
    let (mut i, mut j, mut star, mut retry) = (0, 0, None, 0);
    while j < v.len() {
        if i < p.len() && (p[i] == b'?' || p[i] == v[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == b'*' {
            star = Some(i);
            i += 1;
            retry = j;
        } else if let Some(s) = star {
            i = s + 1;
            retry += 1;
            j = retry;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == b'*' {
        i += 1;
    }
    i == p.len()
}

fn enabled(value: Option<&String>, default: bool) -> bool {
    match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
        Some("0" | "false" | "off" | "no") => false,
        Some(
            "1" | "true" | "on" | "yes" | "compact" | "game" | "game_compact" | "game-compact",
        ) => true,
        _ => default,
    }
}

pub fn suppress_tool_hotkeys(config: &str, process: &str, class: &str) -> bool {
    let mut sections: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut section = String::new();
    for line in config.lines().map(str::trim) {
        if line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_lowercase();
        } else if let Some((k, v)) = line.split_once('=') {
            sections
                .entry(section.clone())
                .or_default()
                .insert(k.trim().to_lowercase(), v.trim().to_string());
        }
    }
    let compatibility = sections.get("compatibility");
    let builtin = enabled(compatibility.and_then(|s| s.get("builtin_game_list")), true);
    let builtin_game = builtin
        && (BUILTIN_GAME_PROCESSES
            .iter()
            .any(|p| wildcard_match(p, process))
            || BUILTIN_GAME_CLASSES.iter()
            .any(|c| c.eq_ignore_ascii_case(class)));
    let configured_game = compatibility
        .and_then(|s| s.get("game_processes"))
        .is_some_and(|list| {
            list.split([',', ';', '\n'])
                .any(|p| !p.trim().is_empty() && wildcard_match(p.trim(), process))
        });
    let exact = sections.get(&format!("app:{}", process.to_lowercase()));
    let app = exact
        .filter(|s| enabled(s.get("enabled"), true))
        .or_else(|| {
            sections.iter().find_map(|(s, v)| {
                s.strip_prefix("app:")
                    .filter(|p| wildcard_match(p, process) && enabled(v.get("enabled"), true))
                    .map(|_| v)
            })
        });
    let profile_game = app.is_some_and(|s| {
        enabled(s.get("game_profile"), false)
            || s.get("game_input_mode")
                .is_some_and(|v| !v.trim().is_empty() && !v.eq_ignore_ascii_case("inherit"))
    });
    let ascii = app.is_some_and(|s| {
        enabled(s.get("ascii_mode"), false)
            || s.get("policy")
                .is_some_and(|p| matches!(p.as_str(), "ascii" | "english"))
    });
    builtin_game || configured_game || profile_game || ascii
}

use kaixin_common::game_rules::{BUILTIN_GAME_PROCESSES, BUILTIN_GAME_CLASSES};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn releases_games_without_an_active_tsf_host() {
        assert!(suppress_tool_hotkeys("", "CS2.EXE", ""));
        assert!(suppress_tool_hotkeys("", "custom.exe", "UnityWndClass"));
        assert!(!suppress_tool_hotkeys("", "notepad.exe", ""));
        assert!(!suppress_tool_hotkeys(
            "[compatibility]\nbuiltin_game_list=0",
            "cs2.exe",
            ""
        ));
        assert!(suppress_tool_hotkeys(
            "[compatibility]\ngame_processes=mygame*.exe",
            "mygame64.exe",
            ""
        ));
        assert!(suppress_tool_hotkeys(
            "[app:custom.exe]\ngame_profile=compact",
            "custom.exe",
            ""
        ));
        assert!(!suppress_tool_hotkeys(
            "[app:custom.exe]\ngame_profile=compact\nenabled=0",
            "custom.exe",
            ""
        ));
        assert!(suppress_tool_hotkeys(
            "[app:custom.exe]\ngame_input_mode=passthrough",
            "custom.exe",
            ""
        ));
        assert!(suppress_tool_hotkeys(
            "[app:editor.exe]\nascii_mode=1",
            "editor.exe",
            ""
        ));
    }
    #[test]
    fn builtin_rules_are_generated_for_both_languages() {
        let cpp = include_str!("../../tsf-tip/include/game_rules.generated.h");
        for value in BUILTIN_GAME_PROCESSES.iter().chain(BUILTIN_GAME_CLASSES.iter()) {
            assert!(cpp.contains(&format!("L\"{value}\"")));
        }
    }
    #[test]
    fn patterns_are_case_insensitive() {
        assert!(wildcard_match("*-shipping.exe", "Game-Win64-Shipping.exe"));
        assert!(!wildcard_match("cs2.exe", "cs2.exe.bak"));
    }
}
