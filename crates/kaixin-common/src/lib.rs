// Generated contracts keep the generator's canonical formatting.
#[rustfmt::skip]
pub mod config_schema;
#[rustfmt::skip]
pub mod game_rules;
#[path = "../../../pinyin-ime/src/shared_rules.rs"]
pub mod shared_rules;
pub fn config_default(section: &str, key: &str) -> Option<&'static str> {
    config_schema::ENTRIES
        .iter()
        .find(|e| e.section.eq_ignore_ascii_case(section) && e.key.eq_ignore_ascii_case(key))
        .map(|e| e.default)
}
pub fn validate_config_value(section: &str, key: &str, value: &str) -> bool {
    let Some(e) = config_schema::ENTRIES
        .iter()
        .find(|e| e.section.eq_ignore_ascii_case(section) && e.key.eq_ignore_ascii_case(key))
    else {
        return true;
    };
    let value = value.trim().to_ascii_lowercase();
    if !e.options.is_empty() {
        return e.options.iter().any(|option| *option == value);
    }
    match e.kind {
        "bool" => matches!(
            value.as_str(),
            "0" | "1" | "true" | "false" | "yes" | "no" | "on" | "off"
        ),
        "integer" => value.parse::<u64>().is_ok(),
        _ => true,
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn defaults_and_validation_preserve_extensions() {
        assert_eq!(
            super::config_default("STYLE", "candidate_density"),
            Some("standard")
        );
        assert!(!super::validate_config_value(
            "compatibility",
            "game_input_mode",
            "invalid"
        ));
        assert!(super::validate_config_value(
            "extension",
            "future",
            "anything"
        ));
        let mut seen = std::collections::HashSet::new();
        for e in super::config_schema::ENTRIES {
            assert!(seen.insert((e.section, e.key)));
            assert!(super::validate_config_value(e.section, e.key, e.default));
        }
    }
}
