use super::*;

const CURRENT_CONFIG_VERSION: usize = pinyin_ime::config_schema::CONFIG_VERSION;
// Configuration values are scalar settings, not lexicon or log storage.  A
// malformed line this large can otherwise be retained verbatim and copied on
// every save (and, historically, on every dirty-check repaint).
const MAX_CONFIG_LINE_BYTES: usize = 64 * 1024;

#[path = "ini_document.rs"]
mod ini_document;
pub(crate) use ini_document::*;

pub(crate) fn shortcuts_text_from_config(config: &IniDoc) -> String {
    config
        .sections
        .get("shortcuts")
        .map(|section| {
            section
                .iter()
                .map(|(key, value)| format!("{key} = {value}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

pub(crate) fn apply_shortcuts_text_to_config(config: &mut IniDoc, text: &str) {
    config.remove_section("shortcuts");
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line.starts_with('#')
            || (line.starts_with(';') && !line.contains('='))
        {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if !key.is_empty() && !value.is_empty() {
            config.set("shortcuts", key, value);
        }
    }
}

pub(crate) fn model_from_config(config: &IniDoc) -> SettingsModel {
    let mut config = config.clone();
    apply_pre_model_config_migrations(&mut config);
    model_from_canonical_config(&config)
}

fn model_from_canonical_config(config: &IniDoc) -> SettingsModel {
    let defaults = SettingsModel::default();
    let config_version = read_config_version(config);
    let mut model = SettingsModel {
        default_ascii: read_bool(config, "input", "default_ascii", defaults.default_ascii),
        global_ascii: read_bool(config, "general", "global_ascii", defaults.global_ascii),
        default_full_shape: read_bool(
            config,
            "input",
            "default_full_shape",
            defaults.default_full_shape,
        ),
        default_chinese_punct: read_bool(
            config,
            "input",
            "default_chinese_punct",
            defaults.default_chinese_punct,
        ),
        curly_punct: read_bool(config, "input", "curly_punct", defaults.curly_punct),
        auto_pair_punct: read_bool(config, "input", "auto_pair_punct", defaults.auto_pair_punct),
        number_fullwidth: read_bool(
            config,
            "input",
            "number_fullwidth",
            defaults.number_fullwidth,
        ),
        chinese_halfwidth: read_bool(
            config,
            "input",
            "chinese_halfwidth",
            defaults.chinese_halfwidth,
        ),
        symbol_fullwidth: read_bool(
            config,
            "input",
            "symbol_fullwidth",
            defaults.symbol_fullwidth,
        ),
        shift_symbol_temporary_ascii: read_bool(
            config,
            "input",
            "shift_symbol_temporary_ascii",
            defaults.shift_symbol_temporary_ascii,
        ),
        date_auto_format: read_bool(
            config,
            "input",
            "date_auto_format",
            defaults.date_auto_format,
        ),
        english_word_input: read_bool(
            config,
            "input",
            "english_word_input",
            defaults.english_word_input,
        ),
        traditional_output: read_bool(
            config,
            "input",
            "traditional_output",
            defaults.traditional_output,
        ),
        default_fuzzy_pinyin: read_bool(
            config,
            "input",
            "default_fuzzy_pinyin",
            defaults.default_fuzzy_pinyin,
        ),
        fuzzy_zh_z: read_bool(config, "fuzzy", "zh_z", defaults.fuzzy_zh_z),
        fuzzy_ch_c: read_bool(config, "fuzzy", "ch_c", defaults.fuzzy_ch_c),
        fuzzy_sh_s: read_bool(config, "fuzzy", "sh_s", defaults.fuzzy_sh_s),
        fuzzy_n_l: read_bool(config, "fuzzy", "n_l", defaults.fuzzy_n_l),
        fuzzy_f_h: read_bool(config, "fuzzy", "f_h", defaults.fuzzy_f_h),
        fuzzy_an_ang: read_bool(config, "fuzzy", "an_ang", defaults.fuzzy_an_ang),
        fuzzy_en_eng: read_bool(config, "fuzzy", "en_eng", defaults.fuzzy_en_eng),
        fuzzy_in_ing: read_bool(config, "fuzzy", "in_ing", defaults.fuzzy_in_ing),
        default_double_pinyin: read_bool(
            config,
            "input",
            "default_double_pinyin",
            defaults.default_double_pinyin,
        ),
        double_pinyin_schema: read_string(
            config,
            "input",
            "double_pinyin_schema",
            &defaults.double_pinyin_schema,
        ),
        jianpin: read_bool(config, "engine", "jianpin", defaults.jianpin),
        mixed_pinyin: read_bool(config, "engine", "mixed_pinyin", defaults.mixed_pinyin),
        mixed_pinyin_aggressive: read_bool(
            config,
            "engine",
            "mixed_pinyin_aggressive",
            defaults.mixed_pinyin_aggressive,
        ),
        learning_sensitivity: read_string(
            config,
            "engine",
            schema_key::LEARNING_SENSITIVITY,
            &defaults.learning_sensitivity,
        ),
        user_hotword_boost: read_string(
            config,
            "engine",
            schema_key::USER_HOTWORD_BOOST,
            &defaults.user_hotword_boost,
        ),
        v_assist: read_bool(config, "engine", "v_assist", defaults.v_assist),
        symbol_toolbox: read_bool(config, "input", "symbol_toolbox", defaults.symbol_toolbox),
        emoji_input: read_bool(config, "input", "emoji_input", defaults.emoji_input),
        u_mode: read_bool(config, "engine", "u_mode", defaults.u_mode),
        custom_shortcuts: shortcuts_text_from_config(config),
        show_status_notifications: read_bool(
            config,
            "engine",
            "show_status_notifications",
            defaults.show_status_notifications,
        ),
        retry_on_failure: read_bool(
            config,
            "engine",
            "retry_on_failure",
            defaults.retry_on_failure,
        ),
        correction_enabled: read_bool(config, "correction", "enabled", defaults.correction_enabled),
        inline_preedit: read_bool(config, "style", "inline_preedit", defaults.inline_preedit),
        enhanced_position: read_bool(
            config,
            "style",
            "enhanced_position",
            defaults.enhanced_position,
        ),
        paging_on_scroll: read_bool(
            config,
            "style",
            "paging_on_scroll",
            defaults.paging_on_scroll,
        ),
        candidate_page_size: read_usize(
            config,
            "style",
            "candidate_page_size",
            defaults.candidate_page_size,
        )
        .clamp(3, 9),
        candidate_horizontal: read_bool(
            config,
            "style",
            "candidate_horizontal",
            defaults.candidate_horizontal,
        ),
        candidate_horizontal_count: read_usize(
            config,
            "style",
            "candidate_horizontal_count",
            defaults.candidate_horizontal_count,
        )
        .clamp(3, 9),
        candidate_horizontal_compact: read_bool(
            config,
            "style",
            "candidate_horizontal_compact",
            defaults.candidate_horizontal_compact,
        ),
        candidate_font_size: read_usize(
            config,
            "style",
            "candidate_font_size",
            defaults.candidate_font_size,
        )
        .clamp(14, 28),
        candidate_opacity: read_usize(
            config,
            "style",
            "candidate_opacity",
            defaults.candidate_opacity,
        )
        .clamp(90, 100),
        candidate_reduce_motion: read_bool(
            config,
            "style",
            "candidate_reduce_motion",
            defaults.candidate_reduce_motion,
        ),
        candidate_font_weight: read_usize(
            config,
            "style",
            "candidate_font_weight",
            defaults.candidate_font_weight,
        )
        .clamp(300, 700),
        candidate_selected_font_weight: read_usize(
            config,
            "style",
            "candidate_selected_font_weight",
            defaults.candidate_selected_font_weight,
        )
        .clamp(400, 800),
        candidate_label_font_weight: read_usize(
            config,
            "style",
            "candidate_label_font_weight",
            defaults.candidate_label_font_weight,
        )
        .clamp(400, 800),
        candidate_chip_font_weight: read_usize(
            config,
            "style",
            "candidate_chip_font_weight",
            defaults.candidate_chip_font_weight,
        )
        .clamp(350, 700),
        candidate_font_file: {
            let family = read_string(
                config,
                "style",
                "candidate_font_file",
                DEFAULT_CANDIDATE_FONT_FAMILY,
            );
            if family.trim().is_empty() {
                DEFAULT_CANDIDATE_FONT_FAMILY.to_owned()
            } else {
                family
            }
        },
        candidate_skin_file: read_string(config, "style", "candidate_skin_file", ""),
        theme: read_string(
            config,
            schema_section::STYLE,
            schema_key::THEME,
            &defaults.theme,
        ),
        candidate_material: read_string(
            config,
            schema_section::STYLE,
            schema_key::CANDIDATE_MATERIAL,
            &defaults.candidate_material,
        ),
        candidate_density: read_string(
            config,
            schema_section::STYLE,
            schema_key::CANDIDATE_DENSITY,
            &defaults.candidate_density,
        ),
        candidate_layout_variant: read_string(
            config,
            schema_section::STYLE,
            schema_key::CANDIDATE_LAYOUT_VARIANT,
            &defaults.candidate_layout_variant,
        ),
        candidate_vertical_layout_variant: read_string(
            config,
            schema_section::STYLE,
            schema_key::CANDIDATE_VERTICAL_LAYOUT_VARIANT,
            config
                .get(schema_section::STYLE, schema_key::CANDIDATE_LAYOUT_VARIANT)
                .unwrap_or(&defaults.candidate_vertical_layout_variant),
        ),
        candidate_horizontal_layout_variant: read_string(
            config,
            schema_section::STYLE,
            schema_key::CANDIDATE_HORIZONTAL_LAYOUT_VARIANT,
            config
                .get(schema_section::STYLE, schema_key::CANDIDATE_LAYOUT_VARIANT)
                .unwrap_or(&defaults.candidate_horizontal_layout_variant),
        ),
        candidate_topmost: read_bool(
            config,
            "style",
            "candidate_topmost",
            defaults.candidate_topmost,
        ),
        show_candidate_reading: read_bool(
            config,
            "style",
            "show_candidate_reading",
            defaults.show_candidate_reading,
        ),
        show_candidate_score: read_bool(
            config,
            "style",
            "show_candidate_score",
            defaults.show_candidate_score,
        ),
        highlight_typo_candidates: read_bool(
            config,
            schema_section::STYLE,
            schema_key::HIGHLIGHT_TYPO_CANDIDATES,
            defaults.highlight_typo_candidates,
        ),
        show_candidate_source: read_bool(
            config,
            schema_section::STYLE,
            schema_key::SHOW_CANDIDATE_SOURCE,
            defaults.show_candidate_source,
        ),
        show_mode_in_candidate_header: read_bool(
            config,
            "style",
            "show_mode_in_candidate_header",
            defaults.show_mode_in_candidate_header,
        ),
        candidate_abbreviate_length: read_usize(
            config,
            "style",
            "candidate_abbreviate_length",
            defaults.candidate_abbreviate_length,
        )
        .clamp(16, 256),
        cn_en_hotkey: read_string(config, "input", "cn_en_hotkey", &defaults.cn_en_hotkey),
        full_shape_hotkey: read_bool(
            config,
            "input",
            "full_shape_hotkey",
            defaults.full_shape_hotkey,
        ),
        punct_hotkey: read_bool(config, "input", "punct_hotkey", defaults.punct_hotkey),
        fuzzy_hotkey: read_bool(config, "input", "fuzzy_hotkey", defaults.fuzzy_hotkey),
        double_pinyin_hotkey: read_bool(
            config,
            "input",
            "double_pinyin_hotkey",
            defaults.double_pinyin_hotkey,
        ),
        shift_tap_hotkey: read_bool(
            config,
            "input",
            "shift_tap_hotkey",
            defaults.shift_tap_hotkey,
        ),
        candidate_number_select: read_bool(
            config,
            "input",
            "candidate_number_select",
            defaults.candidate_number_select,
        ),
        candidate_left_click: read_bool(
            config,
            "input",
            "candidate_left_click",
            defaults.candidate_left_click,
        ),
        candidate_right_click: read_bool(
            config,
            "input",
            "candidate_right_click",
            defaults.candidate_right_click,
        ),
        page_minus_equal: read_bool(
            config,
            "input",
            "page_minus_equal",
            defaults.page_minus_equal,
        ),
        page_comma_period: read_bool(
            config,
            "input",
            "page_comma_period",
            defaults.page_comma_period,
        ),
        page_pgup_pgdn: read_bool(config, "input", "page_pgup_pgdn", defaults.page_pgup_pgdn),
        screenshot_hotkey: read_string(config, "screenshot", "hotkey", &defaults.screenshot_hotkey),
        screenshot_auto_save: read_bool(
            config,
            "screenshot",
            "auto_save",
            defaults.screenshot_auto_save,
        ),
        screenshot_copy_after_capture: read_bool(
            config,
            "screenshot",
            "copy_after_capture",
            defaults.screenshot_copy_after_capture,
        ),
        screenshot_ocr_after_capture: read_bool(
            config,
            "screenshot",
            "ocr_after_capture",
            defaults.screenshot_ocr_after_capture,
        ),
        screenshot_translate_after_capture: read_bool(
            config,
            "screenshot",
            "translate_after_capture",
            defaults.screenshot_translate_after_capture,
        ),
        screenshot_save_dir: read_string(config, "screenshot", "save_dir", ""),
        screenshot_silent_copy_enabled: read_bool(
            config,
            "screenshot",
            "silent_copy_enabled",
            defaults.screenshot_silent_copy_enabled,
        ),
        screenshot_silent_copy_dir: read_string(config, "screenshot", "silent_copy_dir", ""),
        screenshot_name_pattern: read_string(
            config,
            "screenshot",
            "name_pattern",
            &defaults.screenshot_name_pattern,
        ),
        screenshot_date_subdirs: read_bool(
            config,
            "screenshot",
            "date_subdirs",
            defaults.screenshot_date_subdirs,
        ),
        screenshot_conflict_strategy: read_string(
            config,
            "screenshot",
            "conflict_strategy",
            &defaults.screenshot_conflict_strategy,
        ),
        screenshot_format: read_string(config, "screenshot", "format", &defaults.screenshot_format),
        screenshot_mode: read_string(config, "screenshot", "mode", &defaults.screenshot_mode),
        screenshot_confirm_on_release: read_bool(
            config,
            "screenshot",
            "confirm_on_release",
            defaults.screenshot_confirm_on_release,
        ),
        screenshot_show_instructions: read_bool(
            config,
            "screenshot",
            "show_instructions",
            defaults.screenshot_show_instructions,
        ),
        clipboard_hotkey: read_string(config, "clipboard", "hotkey", &defaults.clipboard_hotkey),
        settings_hotkey: read_string(
            config,
            "tools",
            "settings_hotkey",
            &defaults.settings_hotkey,
        ),
        handwrite_hotkey: read_string(
            config,
            "tools",
            "handwrite_hotkey",
            &defaults.handwrite_hotkey,
        ),
        ocr_hotkey: read_string(config, "tools", "ocr_hotkey", &defaults.ocr_hotkey),
        ocr_translate_hotkey: read_string(
            config,
            "tools",
            "ocr_translate_hotkey",
            &defaults.ocr_translate_hotkey,
        ),
        ocr_result_action: read_string(
            config,
            "tools",
            "ocr_result_action",
            &defaults.ocr_result_action,
        ),
        ocr_keep_alive: read_bool(config, "ocr", "keep_alive", defaults.ocr_keep_alive),
        ocr_profile: read_string(config, "ocr", "profile", &defaults.ocr_profile),
        ocr_translate_keep_window: read_bool(
            config,
            "tools",
            "ocr_translate_keep_window",
            defaults.ocr_translate_keep_window,
        ),
        ocr_screenshot_auto_save: read_bool(
            config,
            "ocr",
            "screenshot_auto_save",
            defaults.ocr_screenshot_auto_save,
        ),
        ocr_screenshot_save_dir: read_string(config, "ocr", "screenshot_save_dir", ""),
        ocr_screenshot_name_pattern: read_string(
            config,
            "ocr",
            "screenshot_name_pattern",
            &defaults.ocr_screenshot_name_pattern,
        ),
        translate_target_language: read_string(
            config,
            "tools",
            "translate_target_language",
            "auto-opposite",
        ),
        wintranslator_path: read_string(config, "tools", "wintranslator_path", ""),
        translate_result_action: read_string(
            config,
            "tools",
            "translate_result_action",
            &defaults.translate_result_action,
        ),
        translate_hotkey: read_string(
            config,
            "tools",
            "translate_hotkey",
            &defaults.translate_hotkey,
        ),
        traditional_hotkey: read_string(
            config,
            "input",
            "traditional_hotkey",
            &defaults.traditional_hotkey,
        ),
        game_mode_hotkey: read_string(
            config,
            "input",
            "game_mode_hotkey",
            &defaults.game_mode_hotkey,
        ),
        temporary_ascii_hotkey: read_string(
            config,
            "input",
            "temporary_ascii_hotkey",
            &defaults.temporary_ascii_hotkey,
        ),
        clipboard_background_enabled: read_bool(
            config,
            "clipboard",
            "background_enabled",
            defaults.clipboard_background_enabled,
        ),
        clipboard_max_history_items: read_usize(
            config,
            "clipboard",
            "max_history_items",
            defaults.clipboard_max_history_items,
        )
        .clamp(0, 300),
        clipboard_max_pinned_items: read_usize(
            config,
            "clipboard",
            "max_pinned_items",
            defaults.clipboard_max_pinned_items,
        )
        .clamp(0, 100),
        clipboard_max_text_utf16_units: read_usize(
            config,
            "clipboard",
            "max_text_utf16_units",
            defaults.clipboard_max_text_utf16_units,
        )
        .clamp(20, 20_000),
        clipboard_max_age_days: read_usize(
            config,
            "clipboard",
            "max_age_days",
            defaults.clipboard_max_age_days,
        )
        .clamp(0, 3650),
        clipboard_candidate_preview_enabled: read_bool(
            config,
            "clipboard",
            "candidate_preview_enabled",
            defaults.clipboard_candidate_preview_enabled,
        ),
        clipboard_record_source_app: read_bool(
            config,
            "clipboard",
            "record_source_app",
            defaults.clipboard_record_source_app,
        ),
        clipboard_pinned_respects_max_age: read_bool(
            config,
            "clipboard",
            "pinned_respects_max_age",
            defaults.clipboard_pinned_respects_max_age,
        ),
        game_input_mode: normalize_game_input_mode(
            &read_string(config, "compatibility", "game_input_mode", "manual"),
            false,
        ),
        game_chat: game_chat_options_from_values(config.sections.get("compatibility"), false),
        fullscreen_detection: read_bool(
            config,
            schema_section::COMPATIBILITY,
            schema_key::FULLSCREEN_DETECTION,
            defaults.fullscreen_detection,
        ),
        fullscreen_policy: read_string(
            config,
            schema_section::COMPATIBILITY,
            schema_key::FULLSCREEN_POLICY,
            &defaults.fullscreen_policy,
        ),
        commit_transport: read_string(
            config,
            schema_section::COMPATIBILITY,
            schema_key::COMMIT_TRANSPORT,
            &defaults.commit_transport,
        ),
        builtin_game_list: read_bool(
            config,
            schema_section::COMPATIBILITY,
            schema_key::BUILTIN_GAME_LIST,
            defaults.builtin_game_list,
        ),
        auto_suggest_app_options: read_bool(
            config,
            schema_section::COMPATIBILITY,
            schema_key::AUTO_SUGGEST_APP_OPTIONS,
            defaults.auto_suggest_app_options,
        ),
        game_processes: read_string(
            config,
            schema_section::COMPATIBILITY,
            schema_key::GAME_PROCESSES,
            "",
        ),
        privacy_enabled: read_bool(
            config,
            schema_section::PRIVACY,
            schema_key::PRIVACY_ENABLED,
            defaults.privacy_enabled,
        ),
        privacy_never_learn_processes: read_string(
            config,
            schema_section::PRIVACY,
            schema_key::NEVER_LEARN_PROCESSES,
            "",
        ),
        privacy_never_clipboard_processes: read_string(
            config,
            schema_section::PRIVACY,
            schema_key::NEVER_CLIPBOARD_PROCESSES,
            "",
        ),
        privacy_never_candidate_processes: read_string(
            config,
            schema_section::PRIVACY,
            schema_key::NEVER_CANDIDATE_PROCESSES,
            "",
        ),
        app_rules: app_rules_from_config(config),
        compat_rules: Vec::new(),
        show_notifications: read_string(
            config,
            "general",
            "show_notifications",
            &defaults.show_notifications,
        ),
        show_notifications_time: read_usize(
            config,
            "general",
            "show_notifications_time",
            defaults.show_notifications_time,
        )
        .clamp(500, 5000),
        log_level: read_string(config, "diagnostics", "log_level", &defaults.log_level),
        w_single_lm: read_f64(config, "rank", "w_single_lm", defaults.w_single_lm).clamp(0.0, 1.0),
        w_phrase_path: read_f64(config, "rank", "w_phrase_path", defaults.w_phrase_path)
            .clamp(0.0, 1.0),
        lm_single_scale: read_f64(config, "rank", "lm_single_scale", defaults.lm_single_scale)
            .clamp(0.1, 30.0),
        prefix_cache_capacity: read_usize(
            config,
            schema_section::ENGINE,
            schema_key::PREFIX_CACHE_CAPACITY,
            defaults.prefix_cache_capacity,
        )
        .clamp(8, 512),
        final_lookup_cache_capacity: read_usize(
            config,
            schema_section::ENGINE,
            schema_key::FINAL_LOOKUP_CACHE_CAPACITY,
            defaults.final_lookup_cache_capacity,
        )
        .clamp(8, 512),
        short_lookup_cache_capacity: read_usize(
            config,
            schema_section::ENGINE,
            schema_key::SHORT_LOOKUP_CACHE_CAPACITY,
            defaults.short_lookup_cache_capacity,
        )
        .clamp(8, 512),
        long_lookup_soft_budget_ms: read_usize(
            config,
            schema_section::ENGINE,
            schema_key::LONG_LOOKUP_SOFT_BUDGET_MS,
            defaults.long_lookup_soft_budget_ms,
        )
        .clamp(1, 50),
        long_lookup_min_first_batch_candidates: read_usize(
            config,
            schema_section::ENGINE,
            schema_key::LONG_LOOKUP_MIN_FIRST_BATCH_CANDIDATES,
            defaults.long_lookup_min_first_batch_candidates,
        )
        .clamp(1, 128),
        lexicon_tags: read_lexicon_tags(config),
    };
    if config_version < 4 {
        model.screenshot_auto_save = true;
    }
    if config_version < 5 && model.long_lookup_soft_budget_ms == 4 {
        model.long_lookup_soft_budget_ms = defaults.long_lookup_soft_budget_ms;
    }
    if config_version < 6 {
        model.screenshot_copy_after_capture = true;
    }
    if config_version < 7 {
        // Earlier builds enabled this by default, which made the ordinary
        // screenshot command unexpectedly launch OCR after an upgrade.
        // Treat that old default as opt-in only from this version forward.
        model.screenshot_ocr_after_capture = false;
        model.screenshot_translate_after_capture = false;
    }
    if config_version < 11 && model.screenshot_name_pattern.trim() == "{datetime}" {
        // The former default only had one-second precision. Upgrade that exact
        // default to a millisecond timestamp while preserving custom patterns.
        model.screenshot_name_pattern = defaults.screenshot_name_pattern.clone();
    }
    model.compat_rules = compat_rules_from_config(config, &model.game_processes);
    normalize_combo_values(&mut model);
    model
}

pub(crate) fn apply_model_to_config(config: &mut IniDoc, model: &SettingsModel) {
    let mut model = model.clone();
    sync_compat_rules_to_legacy_fields(&mut model);
    config.remove_section("sharex");
    config.remove_key("screenshot", "backend");
    config.remove_key("screenshot", "open_editor_after_capture");
    config.set(
        "general",
        "config_version",
        CURRENT_CONFIG_VERSION.to_string(),
    );
    config.set("general", "global_ascii", bool_text(model.global_ascii));
    config.set(
        "general",
        "show_notifications",
        model.show_notifications.trim(),
    );
    config.set(
        "general",
        "show_notifications_time",
        model.show_notifications_time.to_string(),
    );
    config.set("diagnostics", "log_level", model.log_level.trim());

    config.set("input", "default_ascii", bool_text(model.default_ascii));
    config.set(
        "input",
        "default_full_shape",
        bool_text(model.default_full_shape),
    );
    config.set(
        "input",
        "default_chinese_punct",
        bool_text(model.default_chinese_punct),
    );
    config.set("input", "curly_punct", bool_text(model.curly_punct));
    config.set("input", "auto_pair_punct", bool_text(model.auto_pair_punct));
    config.set(
        "input",
        "number_fullwidth",
        bool_text(model.number_fullwidth),
    );
    config.set(
        "input",
        "symbol_fullwidth",
        bool_text(model.symbol_fullwidth),
    );
    config.set(
        "input",
        "chinese_halfwidth",
        bool_text(model.chinese_halfwidth),
    );
    config.set(
        "input",
        "shift_symbol_temporary_ascii",
        bool_text(model.shift_symbol_temporary_ascii),
    );
    config.set(
        "input",
        "date_auto_format",
        bool_text(model.date_auto_format),
    );
    config.set(
        "input",
        "english_word_input",
        bool_text(model.english_word_input),
    );
    config.set(
        "input",
        "traditional_output",
        bool_text(model.traditional_output),
    );
    config.set(
        "input",
        "default_fuzzy_pinyin",
        bool_text(model.default_fuzzy_pinyin),
    );
    config.set("fuzzy", "zh_z", bool_text(model.fuzzy_zh_z));
    config.set("fuzzy", "ch_c", bool_text(model.fuzzy_ch_c));
    config.set("fuzzy", "sh_s", bool_text(model.fuzzy_sh_s));
    config.set("fuzzy", "n_l", bool_text(model.fuzzy_n_l));
    config.set("fuzzy", "f_h", bool_text(model.fuzzy_f_h));
    config.set("fuzzy", "an_ang", bool_text(model.fuzzy_an_ang));
    config.set("fuzzy", "en_eng", bool_text(model.fuzzy_en_eng));
    config.set("fuzzy", "in_ing", bool_text(model.fuzzy_in_ing));
    config.set(
        "input",
        "default_double_pinyin",
        bool_text(model.default_double_pinyin),
    );
    config.set(
        "input",
        "double_pinyin_schema",
        model.double_pinyin_schema.trim(),
    );
    config.set("input", "cn_en_hotkey", model.cn_en_hotkey.trim());
    config.set(
        "input",
        "full_shape_hotkey",
        bool_text(model.full_shape_hotkey),
    );
    config.set("input", "punct_hotkey", bool_text(model.punct_hotkey));
    config.set("input", "fuzzy_hotkey", bool_text(model.fuzzy_hotkey));
    config.set(
        "input",
        "double_pinyin_hotkey",
        bool_text(model.double_pinyin_hotkey),
    );
    config.set(
        "input",
        "shift_tap_hotkey",
        bool_text(model.shift_tap_hotkey),
    );
    config.set(
        "input",
        "candidate_number_select",
        bool_text(model.candidate_number_select),
    );
    config.set(
        "input",
        "candidate_left_click",
        bool_text(model.candidate_left_click),
    );
    config.set(
        "input",
        "candidate_right_click",
        bool_text(model.candidate_right_click),
    );
    config.set(
        "input",
        "page_minus_equal",
        bool_text(model.page_minus_equal),
    );
    config.set(
        "input",
        "page_comma_period",
        bool_text(model.page_comma_period),
    );
    config.set("input", "page_pgup_pgdn", bool_text(model.page_pgup_pgdn));
    config.set(
        "input",
        "traditional_hotkey",
        normalized_hotkey_value(&model.traditional_hotkey, "F"),
    );
    config.set(
        "input",
        "game_mode_hotkey",
        normalized_hotkey_value(&model.game_mode_hotkey, "G"),
    );
    config.set(
        "input",
        "temporary_ascii_hotkey",
        normalized_hotkey_value(&model.temporary_ascii_hotkey, "Space"),
    );

    config.set("engine", "jianpin", bool_text(model.jianpin));
    config.set("engine", "mixed_pinyin", bool_text(model.mixed_pinyin));
    config.set(
        "engine",
        "mixed_pinyin_aggressive",
        bool_text(model.mixed_pinyin_aggressive),
    );
    config.set(
        "engine",
        schema_key::LEARNING_SENSITIVITY,
        &model.learning_sensitivity,
    );
    config.set(
        "engine",
        schema_key::USER_HOTWORD_BOOST,
        &model.user_hotword_boost,
    );
    config.set("engine", "v_assist", bool_text(model.v_assist));
    config.set("input", "symbol_toolbox", bool_text(model.symbol_toolbox));
    config.set("input", "emoji_input", bool_text(model.emoji_input));
    config.set("engine", "u_mode", bool_text(model.u_mode));
    apply_shortcuts_text_to_config(config, &model.custom_shortcuts);
    config.set(
        "engine",
        "show_status_notifications",
        bool_text(model.show_status_notifications),
    );
    config.set(
        "engine",
        "retry_on_failure",
        bool_text(model.retry_on_failure),
    );
    config.set("correction", "enabled", bool_text(model.correction_enabled));

    config.set("style", "inline_preedit", bool_text(model.inline_preedit));
    config.set(
        "style",
        "enhanced_position",
        bool_text(model.enhanced_position),
    );
    config.set(
        "style",
        "paging_on_scroll",
        bool_text(model.paging_on_scroll),
    );
    config.set(
        "style",
        "candidate_page_size",
        model.candidate_page_size.to_string(),
    );
    config.set(
        "style",
        "candidate_horizontal",
        bool_text(model.candidate_horizontal),
    );
    config.set(
        "style",
        "candidate_horizontal_count",
        model.candidate_horizontal_count.to_string(),
    );
    config.set(
        "style",
        "candidate_horizontal_compact",
        bool_text(model.candidate_horizontal_compact),
    );
    config.set(
        "style",
        "candidate_font_size",
        model.candidate_font_size.to_string(),
    );
    config.set(
        "style",
        "candidate_opacity",
        model.candidate_opacity.to_string(),
    );
    config.set(
        "style",
        "candidate_reduce_motion",
        bool_text(model.candidate_reduce_motion),
    );
    config.set(
        "style",
        "candidate_font_weight",
        model.candidate_font_weight.to_string(),
    );
    config.set(
        "style",
        "candidate_selected_font_weight",
        model.candidate_selected_font_weight.to_string(),
    );
    config.set(
        "style",
        "candidate_label_font_weight",
        model.candidate_label_font_weight.to_string(),
    );
    config.set(
        "style",
        "candidate_chip_font_weight",
        model.candidate_chip_font_weight.to_string(),
    );
    config.set(
        "style",
        "candidate_font_file",
        model.candidate_font_file.trim(),
    );
    config.set(
        "style",
        "candidate_skin_file",
        model.candidate_skin_file.trim(),
    );
    config.set(schema_section::STYLE, schema_key::THEME, model.theme.trim());
    config.set(
        schema_section::STYLE,
        schema_key::CANDIDATE_MATERIAL,
        model.candidate_material.trim(),
    );
    config.set(
        schema_section::STYLE,
        schema_key::CANDIDATE_DENSITY,
        model.candidate_density.trim(),
    );
    config.remove_key(schema_section::STYLE, schema_key::CANDIDATE_LAYOUT_VARIANT);
    config.set(
        schema_section::STYLE,
        schema_key::CANDIDATE_VERTICAL_LAYOUT_VARIANT,
        model.candidate_vertical_layout_variant.trim(),
    );
    config.set(
        schema_section::STYLE,
        schema_key::CANDIDATE_HORIZONTAL_LAYOUT_VARIANT,
        model.candidate_horizontal_layout_variant.trim(),
    );
    config.set(
        "style",
        "candidate_topmost",
        bool_text(model.candidate_topmost),
    );
    config.set(
        "style",
        "show_candidate_reading",
        bool_text(model.show_candidate_reading),
    );
    config.set(
        "style",
        "show_candidate_score",
        bool_text(model.show_candidate_score),
    );
    config.set(
        schema_section::STYLE,
        schema_key::HIGHLIGHT_TYPO_CANDIDATES,
        bool_text(model.highlight_typo_candidates),
    );
    config.set(
        schema_section::STYLE,
        schema_key::SHOW_CANDIDATE_SOURCE,
        bool_text(model.show_candidate_source),
    );
    config.set(
        "style",
        "show_mode_in_candidate_header",
        bool_text(model.show_mode_in_candidate_header),
    );
    config.set(
        "style",
        "candidate_abbreviate_length",
        model.candidate_abbreviate_length.to_string(),
    );

    config.set(
        "screenshot",
        "hotkey",
        normalized_hotkey_value(&model.screenshot_hotkey, "A"),
    );
    config.set(
        "screenshot",
        "auto_save",
        bool_text(model.screenshot_auto_save),
    );
    config.set(
        "screenshot",
        "copy_after_capture",
        bool_text(model.screenshot_copy_after_capture),
    );
    config.set(
        "screenshot",
        "ocr_after_capture",
        bool_text(model.screenshot_ocr_after_capture),
    );
    config.set(
        "screenshot",
        "translate_after_capture",
        bool_text(model.screenshot_translate_after_capture),
    );
    config.set("screenshot", "save_dir", model.screenshot_save_dir.trim());
    config.set(
        "screenshot",
        "silent_copy_enabled",
        bool_text(model.screenshot_silent_copy_enabled),
    );
    config.set(
        "screenshot",
        "silent_copy_dir",
        model.screenshot_silent_copy_dir.trim(),
    );
    config.set(
        "screenshot",
        "name_pattern",
        model.screenshot_name_pattern.trim(),
    );
    config.set(
        "screenshot",
        "date_subdirs",
        bool_text(model.screenshot_date_subdirs),
    );
    config.set(
        "screenshot",
        "conflict_strategy",
        model.screenshot_conflict_strategy.trim(),
    );
    config.set("screenshot", "format", model.screenshot_format.trim());
    config.set("screenshot", "mode", model.screenshot_mode.trim());
    config.set(
        "screenshot",
        "confirm_on_release",
        bool_text(model.screenshot_confirm_on_release),
    );
    config.set(
        "screenshot",
        "show_instructions",
        bool_text(model.screenshot_show_instructions),
    );
    config.set(
        "clipboard",
        "hotkey",
        normalized_hotkey_value(&model.clipboard_hotkey, "V"),
    );
    config.set(
        "tools",
        "settings_hotkey",
        normalized_hotkey_value(&model.settings_hotkey, "Comma"),
    );
    config.set(
        "tools",
        "handwrite_hotkey",
        normalized_hotkey_value(&model.handwrite_hotkey, "H"),
    );
    config.set(
        "tools",
        "ocr_hotkey",
        normalized_hotkey_value(&model.ocr_hotkey, "O"),
    );
    config.set(
        "tools",
        "ocr_translate_hotkey",
        normalized_hotkey_value(&model.ocr_translate_hotkey, "Y"),
    );
    config.set("tools", "ocr_result_action", model.ocr_result_action.trim());
    config.set("ocr", "keep_alive", bool_text(model.ocr_keep_alive));
    config.set("ocr", "profile", model.ocr_profile.trim());
    config.set(
        "tools",
        "ocr_translate_keep_window",
        bool_text(model.ocr_translate_keep_window),
    );
    config.set(
        "ocr",
        "screenshot_auto_save",
        bool_text(model.ocr_screenshot_auto_save),
    );
    config.set(
        "ocr",
        "screenshot_save_dir",
        model.ocr_screenshot_save_dir.trim(),
    );
    config.set(
        "ocr",
        "screenshot_name_pattern",
        model.ocr_screenshot_name_pattern.trim(),
    );
    config.set(
        "tools",
        "wintranslator_path",
        model.wintranslator_path.trim(),
    );
    config.set(
        "tools",
        "translate_target_language",
        model.translate_target_language.trim(),
    );
    config.set(
        "tools",
        "translate_result_action",
        model.translate_result_action.trim(),
    );
    config.set(
        "tools",
        "translate_hotkey",
        normalized_hotkey_value(&model.translate_hotkey, "T"),
    );
    config.set(
        "clipboard",
        "background_enabled",
        bool_text(model.clipboard_background_enabled),
    );
    config.set(
        "clipboard",
        "max_history_items",
        model.clipboard_max_history_items.to_string(),
    );
    config.set(
        "clipboard",
        "max_pinned_items",
        model.clipboard_max_pinned_items.to_string(),
    );
    config.set(
        "clipboard",
        "max_text_utf16_units",
        model.clipboard_max_text_utf16_units.to_string(),
    );
    config.set(
        "clipboard",
        "max_age_days",
        model.clipboard_max_age_days.to_string(),
    );
    config.set(
        "clipboard",
        "candidate_preview_enabled",
        bool_text(model.clipboard_candidate_preview_enabled),
    );
    config.set(
        "clipboard",
        "record_source_app",
        bool_text(model.clipboard_record_source_app),
    );
    config.set(
        "clipboard",
        "pinned_respects_max_age",
        bool_text(model.clipboard_pinned_respects_max_age),
    );

    config.set(
        "compatibility",
        "game_enter_behavior",
        normalize_game_enter_behavior(&model.game_chat.enter_behavior, false),
    );
    config.set(
        "compatibility",
        "game_auto_uia",
        normalize_game_option_bool(&model.game_chat.auto_uia, false),
    );
    config.set(
        "compatibility",
        "game_status_indicator",
        normalize_game_option_bool(&model.game_chat.status_indicator, false),
    );
    config.set(
        "compatibility",
        "game_input_mode",
        normalize_game_input_mode(&model.game_input_mode, false),
    );
    config.set(
        schema_section::COMPATIBILITY,
        schema_key::FULLSCREEN_DETECTION,
        bool_text(model.fullscreen_detection),
    );
    config.set(
        schema_section::COMPATIBILITY,
        schema_key::FULLSCREEN_POLICY,
        model.fullscreen_policy.trim(),
    );
    config.set(
        schema_section::COMPATIBILITY,
        schema_key::COMMIT_TRANSPORT,
        model.commit_transport.trim(),
    );
    config.set(
        schema_section::COMPATIBILITY,
        schema_key::BUILTIN_GAME_LIST,
        bool_text(model.builtin_game_list),
    );
    config.set(
        schema_section::COMPATIBILITY,
        schema_key::AUTO_SUGGEST_APP_OPTIONS,
        bool_text(model.auto_suggest_app_options),
    );
    let game_processes = normalize_inline_list(&model.game_processes);
    if game_processes.is_empty() {
        config.remove_key(schema_section::COMPATIBILITY, schema_key::GAME_PROCESSES);
    } else {
        config.set(
            schema_section::COMPATIBILITY,
            schema_key::GAME_PROCESSES,
            game_processes,
        );
    }
    config.set(
        schema_section::PRIVACY,
        schema_key::PRIVACY_ENABLED,
        bool_text(model.privacy_enabled),
    );
    config.set(
        schema_section::PRIVACY,
        schema_key::NEVER_LEARN_PROCESSES,
        normalize_inline_list(&model.privacy_never_learn_processes),
    );
    config.set(
        schema_section::PRIVACY,
        schema_key::NEVER_CLIPBOARD_PROCESSES,
        normalize_inline_list(&model.privacy_never_clipboard_processes),
    );
    config.set(
        schema_section::PRIVACY,
        schema_key::NEVER_CANDIDATE_PROCESSES,
        normalize_inline_list(&model.privacy_never_candidate_processes),
    );

    config.set("rank", "w_single_lm", format!("{:.3}", model.w_single_lm));
    config.set(
        "rank",
        "w_phrase_path",
        format!("{:.3}", model.w_phrase_path),
    );
    config.set(
        "rank",
        "lm_single_scale",
        format!("{:.3}", model.lm_single_scale),
    );
    config.set(
        schema_section::ENGINE,
        schema_key::PREFIX_CACHE_CAPACITY,
        model.prefix_cache_capacity.to_string(),
    );
    config.set(
        schema_section::ENGINE,
        schema_key::FINAL_LOOKUP_CACHE_CAPACITY,
        model.final_lookup_cache_capacity.to_string(),
    );
    config.set(
        schema_section::ENGINE,
        schema_key::SHORT_LOOKUP_CACHE_CAPACITY,
        model.short_lookup_cache_capacity.to_string(),
    );
    config.set(
        schema_section::ENGINE,
        schema_key::LONG_LOOKUP_SOFT_BUDGET_MS,
        model.long_lookup_soft_budget_ms.to_string(),
    );
    config.set(
        schema_section::ENGINE,
        schema_key::LONG_LOOKUP_MIN_FIRST_BATCH_CANDIDATES,
        model.long_lookup_min_first_batch_candidates.to_string(),
    );

    let active_lexicon_keys = model
        .lexicon_tags
        .keys()
        .map(|tag| format!("lexicon_{}", tag.to_ascii_lowercase()))
        .collect::<BTreeSet<_>>();
    config.retain_section_keys("lexicon", |key| {
        !key.starts_with("lexicon_") || active_lexicon_keys.contains(key)
    });
    for (tag, enabled) in &model.lexicon_tags {
        let key = format!("lexicon_{}", tag.to_ascii_lowercase());
        config.set("lexicon", &key, bool_text(*enabled));
    }

    remove_app_sections(config);
    apply_app_rules(config, &model.app_rules);
}

pub(crate) fn rendered_config_for_model(config: &IniDoc, model: &SettingsModel) -> String {
    let mut config = config.clone();
    apply_model_to_config(&mut config, model);
    config.render()
}

pub(crate) fn read_bool(config: &IniDoc, section: &str, key: &str, default: bool) -> bool {
    match config
        .get(section, key)
        .map(|v| v.trim().to_ascii_lowercase())
    {
        Some(v) if matches!(v.as_str(), "1" | "true" | "yes" | "on" | "enabled") => true,
        Some(v) if matches!(v.as_str(), "0" | "false" | "no" | "off" | "disabled") => false,
        _ => default,
    }
}

pub(crate) fn read_usize(config: &IniDoc, section: &str, key: &str, default: usize) -> usize {
    config
        .get(section, key)
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(default)
}

fn read_config_version(config: &IniDoc) -> usize {
    config
        .get("general", "config_version")
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0)
}

fn apply_pre_model_config_migrations(config: &mut IniDoc) -> bool {
    let mut changed = canonicalize_legacy_app_rule_keys(config);
    changed |= repair_legacy_mojibake_directory(config, "screenshot", "save_dir");
    changed |= repair_legacy_mojibake_directory(config, "screenshot", "silent_copy_dir");
    changed |= repair_legacy_mojibake_directory(config, "ocr", "screenshot_save_dir");
    if config.sections.contains_key("sharex") {
        config.remove_section("sharex");
        changed = true;
    }
    for key in ["backend", "open_editor_after_capture"] {
        if config.get("screenshot", key).is_some() {
            config.remove_key("screenshot", key);
            changed = true;
        }
    }
    changed
}

/// Older builds could save a path after its UTF-8 bytes had already been
/// decoded as the Chinese ANSI code page. Once that damaged text was written
/// back as valid UTF-8, encoding detection could no longer distinguish it
/// from an intentional value. Recover only an invalid/nonexistent directory
/// and only when one unambiguous sibling has the same ASCII parts. This keeps
/// normal user-authored Unicode paths untouched.
fn repair_legacy_mojibake_directory(config: &mut IniDoc, section: &str, key: &str) -> bool {
    let Some(value) = config.get(section, key).map(str::trim) else {
        return false;
    };
    let damaged = PathBuf::from(value);
    if value.is_empty() || damaged.is_dir() {
        return false;
    }
    let Some(damaged_name) = damaged.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !looks_like_legacy_path_mojibake(damaged_name) {
        return false;
    }
    let Some(parent) = damaged.parent().filter(|parent| parent.is_dir()) else {
        return false;
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return false;
    };
    let candidates = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|kind| kind.is_dir())
                .and_then(|_| entry.file_name().into_string().ok())
        })
        .collect::<Vec<_>>();
    let Some(recovered_name) = best_mojibake_directory_match(damaged_name, &candidates) else {
        return false;
    };
    let recovered = parent.join(recovered_name).display().to_string();
    config.set(section, key, recovered);
    true
}

fn looks_like_legacy_path_mojibake(name: &str) -> bool {
    name.contains('?')
        || name.contains('\u{fffd}')
        || name.contains('\u{20ac}')
        || name.contains('\u{95c1}')
}

pub(crate) fn best_mojibake_directory_match<'a>(
    damaged: &str,
    candidates: &'a [String],
) -> Option<&'a str> {
    let damaged_ascii = ascii_path_fingerprint(damaged);
    if damaged_ascii.len() < 6 {
        return None;
    }

    let mut best: Option<(&str, usize)> = None;
    let mut tied = false;
    for candidate in candidates {
        if candidate.is_ascii() {
            continue;
        }
        let candidate_ascii = ascii_path_fingerprint(candidate);
        let distance = ascii_edit_distance(&damaged_ascii, &candidate_ascii);
        let allowed_distance = 2.max(damaged_ascii.len().max(candidate_ascii.len()) / 12);
        if distance > allowed_distance {
            continue;
        }
        match best {
            None => {
                best = Some((candidate, distance));
                tied = false;
            }
            Some((_, best_distance)) if distance < best_distance => {
                best = Some((candidate, distance));
                tied = false;
            }
            Some((_, best_distance)) if distance == best_distance => tied = true,
            _ => {}
        }
    }
    best.filter(|_| !tied).map(|(name, _)| name)
}

fn ascii_path_fingerprint(value: &str) -> Vec<u8> {
    value
        .bytes()
        .filter(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b' '))
        .map(|byte| byte.to_ascii_lowercase())
        .collect()
}

fn ascii_edit_distance(left: &[u8], right: &[u8]) -> usize {
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_byte) in left.iter().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_byte) in right.iter().enumerate() {
            let replace = previous[right_index] + usize::from(left_byte != right_byte);
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(replace);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn config_needs_canonical_rewrite(config: &IniDoc) -> bool {
    read_config_version(config) < CURRENT_CONFIG_VERSION
        || config
            .get(schema_section::STYLE, schema_key::CANDIDATE_LAYOUT_VARIANT)
            .is_some()
        || config.sections.iter().any(|(section, values)| {
            section.starts_with("app:") && app_section_has_noncanonical_keys(values)
        })
}

fn canonicalize_legacy_app_rule_keys(config: &mut IniDoc) -> bool {
    let app_sections = config
        .sections
        .keys()
        .filter(|section| section.starts_with("app:"))
        .cloned()
        .collect::<Vec<_>>();
    let mut changed = false;
    for section in app_sections {
        changed |= canonicalize_legacy_app_rule_key(config, &section, "ascii_mode", &["ascii"]);
        changed |= canonicalize_legacy_app_rule_key(config, &section, "hide_ui", &["hide"]);
        changed |=
            canonicalize_legacy_app_rule_key(config, &section, "inline_preedit", &["inline"]);
        changed |=
            canonicalize_legacy_app_rule_key(config, &section, "enhanced_position", &["position"]);
        changed |=
            canonicalize_legacy_app_rule_key(config, &section, "candidate_topmost", &["topmost"]);
        changed |= canonicalize_legacy_app_rule_key(config, &section, "focus_policy", &["focus"]);
        changed |= canonicalize_legacy_app_rule_key(
            config,
            &section,
            schema_key::COMMIT_TRANSPORT,
            &["commit", "transport"],
        );
        changed |= canonicalize_legacy_app_rule_key(
            config,
            &section,
            schema_key::GAME_PROFILE,
            &["game", "profile", "candidate_profile"],
        );
        changed |= canonicalize_legacy_app_rule_key(
            config,
            &section,
            schema_key::OVERLAY_ANCHOR,
            &["anchor"],
        );
        changed |= canonicalize_legacy_app_rule_key(
            config,
            &section,
            schema_key::OVERLAY_OFFSET_X,
            &["offset_x"],
        );
        changed |= canonicalize_legacy_app_rule_key(
            config,
            &section,
            schema_key::OVERLAY_OFFSET_Y,
            &["offset_y"],
        );
        changed |= canonicalize_legacy_app_rule_key(
            config,
            &section,
            schema_key::OVERLAY_SCALE,
            &["scale"],
        );
        changed |= canonicalize_legacy_app_rule_key(
            config,
            &section,
            schema_key::OVERLAY_MONITOR,
            &["monitor"],
        );
        changed |= canonicalize_legacy_app_rule_key(
            config,
            &section,
            schema_key::OVERLAY_BACKEND,
            &["backend"],
        );
    }
    changed
}

fn canonicalize_legacy_app_rule_key(
    config: &mut IniDoc,
    section: &str,
    canonical_key: &str,
    aliases: &[&str],
) -> bool {
    let mut changed = false;
    if config.get(section, canonical_key).is_none() {
        if let Some(value) = aliases
            .iter()
            .find_map(|alias| config.get(section, alias).map(str::to_string))
        {
            config.set(section, canonical_key, value);
            changed = true;
        }
    }
    for alias in aliases {
        if config.get(section, alias).is_some() {
            config.remove_key(section, alias);
            changed = true;
        }
    }
    changed
}

fn app_section_has_noncanonical_keys(values: &BTreeMap<String, String>) -> bool {
    values.keys().any(|key| {
        !matches!(
            key.as_str(),
            "enabled"
                | "policy"
                | "ascii_mode"
                | "hide_ui"
                | "inline_preedit"
                | "enhanced_position"
                | "candidate_topmost"
                | "focus_policy"
                | "commit_transport"
                | schema_key::GAME_PROFILE
                | schema_key::OVERLAY_ANCHOR
                | schema_key::OVERLAY_OFFSET_X
                | schema_key::OVERLAY_OFFSET_Y
                | schema_key::OVERLAY_SCALE
                | schema_key::OVERLAY_MONITOR
                | schema_key::OVERLAY_BACKEND
        )
    })
}

pub(crate) fn read_f64(config: &IniDoc, section: &str, key: &str, default: f64) -> f64 {
    config
        .get(section, key)
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}

pub(crate) fn read_string(config: &IniDoc, section: &str, key: &str, default: &str) -> String {
    config
        .get(section, key)
        .unwrap_or(default)
        .trim()
        .to_string()
}

pub(crate) fn bool_text(value: bool) -> &'static str {
    if value {
        "1"
    } else {
        "0"
    }
}

pub(crate) fn normalize_combo_values(model: &mut SettingsModel) {
    keep_known(
        &mut model.theme,
        schema_options::THEMES,
        schema_default::THEME,
    );
    keep_known(
        &mut model.candidate_material,
        schema_options::CANDIDATE_MATERIALS,
        schema_default::CANDIDATE_MATERIAL,
    );
    keep_known(
        &mut model.candidate_density,
        schema_options::CANDIDATE_DENSITIES,
        schema_default::CANDIDATE_DENSITY,
    );
    keep_known(
        &mut model.candidate_layout_variant,
        schema_options::CANDIDATE_LAYOUTS,
        schema_default::CANDIDATE_LAYOUT_VARIANT,
    );
    keep_known(
        &mut model.candidate_vertical_layout_variant,
        schema_options::CANDIDATE_LAYOUTS,
        schema_default::CANDIDATE_LAYOUT_VARIANT,
    );
    keep_known(
        &mut model.candidate_horizontal_layout_variant,
        schema_options::CANDIDATE_LAYOUTS,
        schema_default::CANDIDATE_HORIZONTAL_LAYOUT_VARIANT,
    );
    keep_known(&mut model.screenshot_format, &["png", "jpg", "jpeg"], "png");
    if model.screenshot_format == "jpeg" {
        model.screenshot_format = "jpg".to_string();
    }
    if model.screenshot_translate_after_capture {
        model.screenshot_ocr_after_capture = true;
    }
    if model.screenshot_name_pattern.trim().is_empty() {
        model.screenshot_name_pattern = SettingsModel::default().screenshot_name_pattern;
    }
    model.screenshot_conflict_strategy = match model.screenshot_conflict_strategy.trim() {
        "overwrite" => "overwrite".to_string(),
        "increment" => "increment".to_string(),
        _ => "increment".to_string(),
    };
    if model.ocr_screenshot_name_pattern.trim().is_empty() {
        model.ocr_screenshot_name_pattern = SettingsModel::default().ocr_screenshot_name_pattern;
    }
    model.screenshot_hotkey = normalized_hotkey_value(&model.screenshot_hotkey, "A");
    model.clipboard_hotkey = normalized_hotkey_value(&model.clipboard_hotkey, "V");
    model.settings_hotkey = normalized_hotkey_value(&model.settings_hotkey, "Comma");
    model.handwrite_hotkey = normalized_hotkey_value(&model.handwrite_hotkey, "H");
    model.ocr_hotkey = normalized_hotkey_value(&model.ocr_hotkey, "O");
    model.ocr_translate_hotkey = normalized_hotkey_value(&model.ocr_translate_hotkey, "Y");
    keep_known(
        &mut model.ocr_result_action,
        &["show", "copy", "paste"],
        "show",
    );
    keep_known(
        &mut model.ocr_profile,
        &["fast", "balanced", "accurate"],
        "balanced",
    );
    keep_known(
        &mut model.translate_result_action,
        &["show", "copy", "paste"],
        "show",
    );
    model.translate_hotkey = normalized_hotkey_value(&model.translate_hotkey, "T");
    model.traditional_hotkey = normalized_hotkey_value(&model.traditional_hotkey, "F");
    model.game_mode_hotkey = normalized_hotkey_value(&model.game_mode_hotkey, "G");
    model.temporary_ascii_hotkey = normalized_hotkey_value(&model.temporary_ascii_hotkey, "Space");
    keep_known(
        &mut model.cn_en_hotkey,
        &["both", "shift", "ctrl_space", "none"],
        "none",
    );
    keep_known(
        &mut model.fullscreen_policy,
        schema_options::FULLSCREEN_POLICIES,
        schema_default::FULLSCREEN_POLICY,
    );
    keep_known(
        &mut model.commit_transport,
        schema_options::COMMIT_TRANSPORTS,
        schema_default::COMMIT_TRANSPORT,
    );
    keep_known(
        &mut model.learning_sensitivity,
        schema_options::LEARNING_SENSITIVITY,
        schema_default::LEARNING_SENSITIVITY,
    );
    keep_known(
        &mut model.user_hotword_boost,
        schema_options::USER_HOTWORD_BOOST,
        schema_default::USER_HOTWORD_BOOST,
    );
    for rule in &mut model.compat_rules {
        rule.game_chat.enter_behavior =
            normalize_game_enter_behavior(&rule.game_chat.enter_behavior, true);
        rule.game_chat.auto_uia = normalize_game_option_bool(&rule.game_chat.auto_uia, true);
        rule.game_chat.status_indicator =
            normalize_game_option_bool(&rule.game_chat.status_indicator, true);
        rule.commit_transport = normalize_commit_transport_value(&rule.commit_transport, "global");
        rule.overlay_anchor = normalize_overlay_anchor_value(&rule.overlay_anchor);
        rule.overlay_offset_x = rule.overlay_offset_x.clamp(-4000, 4000);
        rule.overlay_offset_y = rule.overlay_offset_y.clamp(-4000, 4000);
        rule.overlay_scale_percent = rule.overlay_scale_percent.clamp(50, 200);
        rule.overlay_monitor = normalize_overlay_monitor_value(&rule.overlay_monitor);
        rule.overlay_backend = normalize_overlay_backend_value(&rule.overlay_backend);
    }
    keep_known(
        &mut model.log_level,
        &["off", "error", "basic", "perf", "verbose"],
        "basic",
    );
}

pub(crate) fn keep_known(value: &mut String, known: &[&str], fallback: &str) {
    let lower = value.trim().to_ascii_lowercase();
    *value = if known.contains(&lower.as_str()) {
        lower
    } else {
        fallback.to_string()
    };
}

pub(crate) fn normalize_commit_transport_value(value: &str, fallback: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "global" | "inherit" | "default" | "" => "global".to_string(),
        "auto" | "game_auto" | "game-auto" | "compat_auto" | "compat-auto" => "auto".to_string(),
        "tsf" | "standard" | "normal" => "tsf".to_string(),
        "clipboard_paste" | "clipboard-paste" | "clipboard" | "paste" | "ctrl_v" | "ctrl-v" => {
            "clipboard_paste".to_string()
        }
        "unicode_sendinput" | "unicode-sendinput" | "unicode" | "sendinput" | "send_input" => {
            "unicode_sendinput".to_string()
        }
        _ => fallback.to_string(),
    }
}

pub(crate) fn normalize_overlay_anchor_value(value: &str) -> String {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "caret" | "cursor" | "text" => "caret".to_string(),
        "top_left" | "upper_left" => "top_left".to_string(),
        "top_center" | "upper_center" => "top_center".to_string(),
        "top_right" | "upper_right" => "top_right".to_string(),
        "bottom_left" | "lower_left" => "bottom_left".to_string(),
        "bottom_center" | "lower_center" => "bottom_center".to_string(),
        "bottom_right" | "lower_right" => "bottom_right".to_string(),
        _ => schema_default::OVERLAY_ANCHOR.to_string(),
    }
}

pub(crate) fn normalize_overlay_backend_value(value: &str) -> String {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "in_process" | "inprocess" | "internal" | "tip" => "in_process".to_string(),
        "external" | "out_of_process" | "outofprocess" | "process" => "external".to_string(),
        _ => schema_default::OVERLAY_BACKEND.to_string(),
    }
}

pub(crate) fn normalize_overlay_monitor_value(value: &str) -> String {
    let trimmed = value.trim();
    let lowered = trimmed.to_ascii_lowercase();
    match lowered.as_str() {
        "" | "auto" | "inherit" | "default" | "current" | "game" => {
            schema_default::OVERLAY_MONITOR.to_string()
        }
        "primary" | "main" => "primary".to_string(),
        _ => lowered
            .parse::<usize>()
            .ok()
            .filter(|index| *index <= 15)
            .map(|index| index.to_string())
            .unwrap_or_else(|| trimmed.to_string()),
    }
}

pub(crate) fn parse_overlay_offset(value: &str) -> i32 {
    value.trim().parse::<i32>().unwrap_or(0).clamp(-4000, 4000)
}

pub(crate) fn parse_overlay_scale_percent(value: &str) -> usize {
    let normalized = value.trim().trim_end_matches('%').trim();
    let Ok(parsed) = normalized.parse::<f64>() else {
        return schema_default::OVERLAY_SCALE_PERCENT;
    };
    let percent = if parsed > 0.0 && parsed <= 4.0 {
        parsed * 100.0
    } else {
        parsed
    };
    percent.round().clamp(50.0, 200.0) as usize
}

pub(crate) fn normalize_inline_list(text: &str) -> String {
    text.lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) fn read_lexicon_tags(config: &IniDoc) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    if let Some(section) = config.sections.get("lexicon") {
        for (key, value) in section {
            if let Some(tag) = key.strip_prefix("lexicon_") {
                out.insert(tag.to_string(), parse_bool_value(value, true));
            }
        }
    }
    out
}

pub(crate) fn merge_discovered_lexicon_tags(model: &mut SettingsModel) {
    let Some(dir) = pinyin_ime::core::default_phrase_lexicon_dir() else {
        model.lexicon_tags.clear();
        return;
    };
    let discovered = pinyin_ime::lexicon_prefs::discover_optional_lexicon_tags_in_lexicon_dir(&dir);
    let discovered_tags = discovered
        .iter()
        .map(|(tag, _)| tag.clone())
        .collect::<BTreeSet<_>>();
    model
        .lexicon_tags
        .retain(|tag, _| discovered_tags.contains(tag));
    for (tag, _) in discovered {
        let enabled = pinyin_ime::lexicon_prefs::default_optional_lexicon_tag_enabled(&tag);
        model.lexicon_tags.entry(tag).or_insert(enabled);
    }
}

pub(crate) fn discover_skin_previews() -> Vec<SkinPreview> {
    discover_skin_theme_paths()
        .into_iter()
        .map(|(key, path)| {
            let value = fs::read_to_string(path)
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .unwrap_or_default();
            let field = |name: &str, fallback: &str| {
                value
                    .get(name)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(fallback)
                    .to_string()
            };
            let number = |name: &str| {
                value
                    .get(name)
                    .and_then(serde_json::Value::as_f64)
                    .map(|value| value as f32)
            };
            SkinPreview {
                display_name: field("name", &key),
                window_bg: field("window_bg", "#F8FAFC"),
                header_bg: field("header_bg", "#F1F5F9"),
                border: field("border", "#CBD5E1"),
                divider: field("divider", "#E2E8F0"),
                item_bg: field("item_bg", "#FFFFFF"),
                item_border: field("item_border", "#E2E8F0"),
                selected_bg: field("selected_bg", "#EAF3FF"),
                selected_border: field("selected_border", "#2563EB"),
                text: field("text", "#0F172A"),
                muted_text: field("muted_text", "#64748B"),
                selected_text: field("selected_text", "#0F172A"),
                selected_muted_text: field("selected_muted_text", "#475569"),
                chip_bg: field("chip_bg", "#F1F5F9"),
                chip_border: field("chip_border", "#E2E8F0"),
                chip_text: field("chip_text", "#334155"),
                outer_pad_y: number("outer_pad_y"),
                header_pad_y: number("header_pad_y"),
                header_gap: number("header_gap"),
                item_gap: number("item_gap"),
                item_pad_y: number("item_pad_y"),
                label_width: number("label_width"),
                comment_gap: number("comment_gap"),
                key,
            }
        })
        .collect()
}

pub(crate) fn discover_skin_theme_paths() -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    let roots = [
        std::env::current_dir().ok(),
        std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf)),
    ];
    for root in roots.into_iter().flatten() {
        for dir in [root.join("skins"), root.join("..").join("skins")] {
            if let Ok(entries) = fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let theme_path = path.join("theme.json");
                    if theme_path.is_file() {
                        if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                            out.entry(name.to_string()).or_insert(theme_path);
                        }
                    }
                }
            }
        }
    }
    out
}

pub(crate) fn app_rules_from_config(config: &IniDoc) -> String {
    let mut lines = Vec::new();
    for (section, values) in &config.sections {
        let Some(app) = section.strip_prefix("app:") else {
            continue;
        };
        let ascii = values.get("ascii_mode").map(String::as_str).unwrap_or("");
        let inline = values
            .get("inline_preedit")
            .map(String::as_str)
            .unwrap_or("");
        let position = values
            .get("enhanced_position")
            .map(String::as_str)
            .unwrap_or("");
        let topmost = values
            .get("candidate_topmost")
            .map(String::as_str)
            .unwrap_or("");
        let focus = values.get("focus_policy").map(String::as_str).unwrap_or("");
        let hide_ui = values.get("hide_ui").map(String::as_str).unwrap_or("");
        let game_profile = values
            .get(schema_key::GAME_PROFILE)
            .map(String::as_str)
            .unwrap_or("");
        let overlay_anchor = values
            .get(schema_key::OVERLAY_ANCHOR)
            .map(String::as_str)
            .unwrap_or("");
        let overlay_offset_x = values
            .get(schema_key::OVERLAY_OFFSET_X)
            .map(String::as_str)
            .unwrap_or("");
        let overlay_offset_y = values
            .get(schema_key::OVERLAY_OFFSET_Y)
            .map(String::as_str)
            .unwrap_or("");
        let overlay_scale = values
            .get(schema_key::OVERLAY_SCALE)
            .map(String::as_str)
            .unwrap_or("");
        let overlay_monitor = values
            .get(schema_key::OVERLAY_MONITOR)
            .map(String::as_str)
            .unwrap_or("");
        let overlay_backend = values
            .get(schema_key::OVERLAY_BACKEND)
            .map(String::as_str)
            .unwrap_or("");
        let commit_transport = values
            .get("commit_transport")
            .map(String::as_str)
            .unwrap_or("");
        let enabled = values.get("enabled").map(String::as_str).unwrap_or("");
        let policy = values.get("policy").map(String::as_str).unwrap_or("");
        let mut parts = vec![app.to_string()];
        if !enabled.is_empty() {
            parts.push(format!("enabled={enabled}"));
        }
        if !policy.is_empty() {
            parts.push(format!("policy={policy}"));
        }
        if !ascii.is_empty() {
            parts.push(format!("ascii={ascii}"));
        }
        if !hide_ui.is_empty() {
            parts.push(format!("hide_ui={hide_ui}"));
        }
        if !inline.is_empty() {
            parts.push(format!("inline={inline}"));
        }
        if !position.is_empty() {
            parts.push(format!("position={position}"));
        }
        if !topmost.is_empty() {
            parts.push(format!("topmost={topmost}"));
        }
        if !focus.is_empty() {
            parts.push(format!("focus={focus}"));
        }
        if !commit_transport.is_empty() {
            parts.push(format!("commit_transport={commit_transport}"));
        }
        if let Some(mode) = values.get("game_input_mode") {
            parts.push(format!(
                "game_input_mode={}",
                normalize_game_input_mode(mode, true)
            ));
        }
        if values.keys().any(|key| {
            key.starts_with("game_chat_")
                || key.starts_with("game_tested_")
                || matches!(
                    key.as_str(),
                    "game_enter_behavior"
                        | "game_auto_uia"
                        | "game_status_indicator"
                        | "overlay_force_ui"
                )
        }) {
            append_game_chat_parts(
                &game_chat_options_from_values(Some(values), true),
                &mut parts,
            );
        }
        if !game_profile.is_empty() {
            parts.push(format!("game_profile={game_profile}"));
        }
        if !overlay_anchor.is_empty() {
            parts.push(format!("overlay_anchor={overlay_anchor}"));
        }
        if !overlay_offset_x.is_empty() {
            parts.push(format!("overlay_offset_x={overlay_offset_x}"));
        }
        if !overlay_offset_y.is_empty() {
            parts.push(format!("overlay_offset_y={overlay_offset_y}"));
        }
        if !overlay_scale.is_empty() {
            parts.push(format!("overlay_scale={overlay_scale}"));
        }
        if !overlay_monitor.is_empty() {
            parts.push(format!("overlay_monitor={overlay_monitor}"));
        }
        if !overlay_backend.is_empty() {
            parts.push(format!("overlay_backend={overlay_backend}"));
        }
        lines.push(parts.join(", "));
    }
    lines.join("\n")
}

pub(crate) fn normalize_game_enter_behavior(value: &str, per_app: bool) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "auto" => "auto",
        "close" => "close",
        "stay" => "stay",
        _ if per_app => "inherit",
        _ => "auto",
    }
    .to_string()
}
pub(crate) fn normalize_game_option_bool(value: &str, per_app: bool) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => "1",
        "0" | "false" | "off" | "no" => "0",
        _ if per_app => "inherit",
        _ => "1",
    }
    .to_string()
}
fn game_chat_options_from_values(
    values: Option<&BTreeMap<String, String>>,
    per_app: bool,
) -> GameChatOptions {
    let mut result = if per_app {
        GameChatOptions::inherited()
    } else {
        GameChatOptions::default()
    };
    let Some(values) = values else {
        return result;
    };
    if let Some(v) = values.get("game_enter_behavior") {
        result.enter_behavior = normalize_game_enter_behavior(v, per_app);
    }
    if let Some(v) = values.get("game_auto_uia") {
        result.auto_uia = normalize_game_option_bool(v, per_app);
    }
    if let Some(v) = values.get("game_status_indicator") {
        result.status_indicator = normalize_game_option_bool(v, per_app);
    }
    if let Some(v) = values.get("game_chat_open_key") {
        result.open_key = v.trim().to_string();
    }
    if let Some(v) = values.get("game_chat_close_key") {
        result.close_key = v.trim().to_string();
    }
    result.force_ui = values
        .get("overlay_force_ui")
        .is_some_and(|v| parse_bool_value(v, false));
    if let Some(v) = values.get("game_tested_display_mode") {
        result.tested_display_mode = v.trim().to_string();
    }
    if let Some(v) = values.get("game_tested_game_version") {
        result.tested_game_version = v.trim().to_string();
    }
    for (key, flag) in [
        ("game_tested_input", &mut result.tested_input),
        ("game_tested_candidate", &mut result.tested_candidate),
        ("game_tested_commit", &mut result.tested_commit),
        ("game_tested_exit", &mut result.tested_exit),
    ] {
        *flag = values.get(key).is_some_and(|v| parse_bool_value(v, false));
    }
    result
}
fn append_game_chat_parts(chat: &GameChatOptions, parts: &mut Vec<String>) {
    if chat.enter_behavior != "inherit" {
        parts.push(format!(
            "game_enter_behavior={}",
            normalize_game_enter_behavior(&chat.enter_behavior, true)
        ));
    }
    if chat.auto_uia != "inherit" {
        parts.push(format!(
            "game_auto_uia={}",
            normalize_game_option_bool(&chat.auto_uia, true)
        ));
    }
    if chat.status_indicator != "inherit" {
        parts.push(format!(
            "game_status_indicator={}",
            normalize_game_option_bool(&chat.status_indicator, true)
        ));
    }
    let clean_key = |v: &str| v.trim().replace([',', '\n', '\r', '='], " ");
    parts.push(format!("game_chat_open_key={}", clean_key(&chat.open_key)));
    parts.push(format!(
        "game_chat_close_key={}",
        clean_key(&chat.close_key)
    ));
    parts.push(format!("overlay_force_ui={}", bool_text(chat.force_ui)));
    if chat.tested_display_mode != "unverified"
        || !chat.tested_game_version.is_empty()
        || chat.tested_input
        || chat.tested_candidate
        || chat.tested_commit
        || chat.tested_exit
    {
        // App-rule text uses commas/newlines as separators, so escape metadata
        // by rejecting those delimiters instead of allowing extra settings.
        let clean = |v: &str| v.replace([',', '\n', '\r', '='], " ");
        parts.push(format!(
            "game_tested_display_mode={}",
            clean(&chat.tested_display_mode)
        ));
        parts.push(format!(
            "game_tested_game_version={}",
            clean(&chat.tested_game_version)
        ));
        for (key, flag) in [
            ("game_tested_input", chat.tested_input),
            ("game_tested_candidate", chat.tested_candidate),
            ("game_tested_commit", chat.tested_commit),
            ("game_tested_exit", chat.tested_exit),
        ] {
            parts.push(format!("{key}={}", bool_text(flag)));
        }
    }
}

pub(crate) fn normalize_game_input_mode(value: &str, per_app: bool) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "manual" => "manual",
        "passthrough" => "passthrough",
        "chinese" => "chinese",
        "auto_text" => "auto_text",
        _ if per_app => "inherit",
        _ => "manual",
    }
    .to_string()
}

pub(crate) fn compat_rules_from_config(config: &IniDoc, game_processes: &str) -> Vec<CompatRule> {
    let mut rules = Vec::new();
    for process in split_inline_list(game_processes) {
        add_compat_rule_unique(&mut rules, &process, CompatRulePolicy::Global);
    }

    for (section, values) in &config.sections {
        let Some(app) = section.strip_prefix("app:") else {
            continue;
        };
        let app = app.trim();
        if app.is_empty() {
            continue;
        }
        let enabled = values
            .get("enabled")
            .map(|value| parse_bool_value(value, true))
            .unwrap_or(true);
        let policy = values
            .get("policy")
            .map(|value| CompatRulePolicy::from_config_name(value))
            .unwrap_or_else(|| compat_policy_from_app_options(values));
        let commit_transport = compat_commit_transport_from_app_options(values);
        let game_profile = compat_game_profile_from_app_options(values);
        upsert_compat_rule(
            &mut rules,
            app,
            enabled,
            policy,
            &commit_transport,
            game_profile,
        );
        if let Some(rule) = rules
            .iter_mut()
            .find(|rule| rule.process.eq_ignore_ascii_case(app))
        {
            rule.game_input_mode = normalize_game_input_mode(
                values
                    .get("game_input_mode")
                    .map(String::as_str)
                    .unwrap_or("inherit"),
                true,
            );
            rule.game_chat = game_chat_options_from_values(Some(values), true);
            apply_overlay_options_to_compat_rule(rule, values);
        }
    }
    rules
}

pub(crate) fn compat_commit_transport_from_app_options(
    values: &BTreeMap<String, String>,
) -> String {
    values
        .get("commit_transport")
        .map(|value| normalize_commit_transport_value(value, "global"))
        .unwrap_or_else(|| "global".to_string())
}

pub(crate) fn compat_game_profile_from_app_options(values: &BTreeMap<String, String>) -> bool {
    values
        .get(schema_key::GAME_PROFILE)
        .or_else(|| values.get("candidate_profile"))
        .or_else(|| values.get("profile"))
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on" | "game" | "compact" | "game_compact" | "game-compact"
            )
        })
}

fn apply_overlay_options_to_compat_rule(rule: &mut CompatRule, values: &BTreeMap<String, String>) {
    rule.overlay_anchor = normalize_overlay_anchor_value(
        values
            .get(schema_key::OVERLAY_ANCHOR)
            .map(String::as_str)
            .unwrap_or(schema_default::OVERLAY_ANCHOR),
    );
    rule.overlay_offset_x = parse_overlay_offset(
        values
            .get(schema_key::OVERLAY_OFFSET_X)
            .map(String::as_str)
            .unwrap_or("0"),
    );
    rule.overlay_offset_y = parse_overlay_offset(
        values
            .get(schema_key::OVERLAY_OFFSET_Y)
            .map(String::as_str)
            .unwrap_or("0"),
    );
    rule.overlay_scale_percent = parse_overlay_scale_percent(
        values
            .get(schema_key::OVERLAY_SCALE)
            .map(String::as_str)
            .unwrap_or("100"),
    );
    rule.overlay_monitor = normalize_overlay_monitor_value(
        values
            .get(schema_key::OVERLAY_MONITOR)
            .map(String::as_str)
            .unwrap_or(schema_default::OVERLAY_MONITOR),
    );
    rule.overlay_backend = normalize_overlay_backend_value(
        values
            .get(schema_key::OVERLAY_BACKEND)
            .map(String::as_str)
            .unwrap_or(schema_default::OVERLAY_BACKEND),
    );
}

pub(crate) fn compat_policy_from_app_options(
    values: &BTreeMap<String, String>,
) -> CompatRulePolicy {
    if let Some(value) = values.get("focus_policy") {
        match value.trim().to_ascii_lowercase().as_str() {
            "strict" | "stable" | "safe" | "focus_strict" => {
                return CompatRulePolicy::FocusStrict;
            }
            "window" | "window_only" | "window-only" | "focus_window" | "context_window" => {
                return CompatRulePolicy::FocusWindow;
            }
            _ => {}
        }
    }
    if values
        .get("hide_ui")
        .is_some_and(|value| parse_bool_value(value, false))
    {
        return CompatRulePolicy::HideUi;
    }
    if values
        .get("ascii_mode")
        .is_some_and(|value| parse_bool_value(value, false))
    {
        return CompatRulePolicy::Ascii;
    }
    if values
        .get("ascii_mode")
        .is_some_and(|value| !parse_bool_value(value, true))
        || values
            .get("hide_ui")
            .is_some_and(|value| !parse_bool_value(value, true))
    {
        return CompatRulePolicy::ShowUi;
    }
    if values
        .get("candidate_topmost")
        .is_some_and(|value| !parse_bool_value(value, true))
    {
        return CompatRulePolicy::DisableTopmost;
    }
    if values
        .get("inline_preedit")
        .is_some_and(|value| !parse_bool_value(value, true))
    {
        return CompatRulePolicy::DisableInlinePreedit;
    }
    if values
        .get("enhanced_position")
        .is_some_and(|value| !parse_bool_value(value, true))
    {
        return CompatRulePolicy::DisableEnhancedPosition;
    }
    CompatRulePolicy::Global
}

pub(crate) fn sync_compat_rules_to_legacy_fields(model: &mut SettingsModel) {
    model.game_processes = model
        .compat_rules
        .iter()
        .filter(|rule| {
            rule.enabled
                && rule.policy == CompatRulePolicy::Global
                && normalize_commit_transport_value(&rule.commit_transport, "global") == "global"
                && rule.game_input_mode == "inherit"
                && !rule.game_profile
        })
        .map(|rule| rule.process.trim())
        .filter(|process| !process.is_empty())
        .collect::<Vec<_>>()
        .join(",");
    model.app_rules = compat_rules_to_app_rules(&model.compat_rules);
}

pub(crate) fn compat_rules_to_app_rules(rules: &[CompatRule]) -> String {
    let mut lines = Vec::new();
    for rule in rules {
        let process = rule.process.trim();
        if process.is_empty() {
            continue;
        }
        let mut parts = vec![
            process.to_string(),
            format!("enabled={}", bool_text(rule.enabled)),
            format!("policy={}", rule.policy.config_name()),
        ];
        if rule.enabled {
            match rule.policy {
                CompatRulePolicy::Global => {}
                CompatRulePolicy::ShowUi => {
                    parts.push("ascii=0".to_string());
                    parts.push("hide_ui=0".to_string());
                    parts.push("topmost=1".to_string());
                }
                CompatRulePolicy::Ascii => parts.push("ascii=1".to_string()),
                CompatRulePolicy::HideUi => parts.push("hide_ui=1".to_string()),
                CompatRulePolicy::DisableTopmost => parts.push("topmost=0".to_string()),
                CompatRulePolicy::DisableInlinePreedit => parts.push("inline=0".to_string()),
                CompatRulePolicy::DisableEnhancedPosition => parts.push("position=0".to_string()),
                CompatRulePolicy::FocusStrict => parts.push("focus=strict".to_string()),
                CompatRulePolicy::FocusWindow => parts.push("focus=window".to_string()),
            }
        }
        let transport = normalize_commit_transport_value(&rule.commit_transport, "global");
        if transport != "global" {
            parts.push(format!("commit_transport={transport}"));
        }
        if rule.game_input_mode != "inherit" {
            parts.push(format!(
                "game_input_mode={}",
                normalize_game_input_mode(&rule.game_input_mode, true)
            ));
        }
        append_game_chat_parts(&rule.game_chat, &mut parts);
        if rule.game_profile {
            parts.push("game_profile=compact".to_string());
        }
        let has_overlay_customization = rule.overlay_anchor != schema_default::OVERLAY_ANCHOR
            || rule.overlay_offset_x != 0
            || rule.overlay_offset_y != 0
            || rule.overlay_scale_percent != schema_default::OVERLAY_SCALE_PERCENT
            || rule.overlay_monitor != schema_default::OVERLAY_MONITOR
            || rule.overlay_backend != schema_default::OVERLAY_BACKEND;
        if rule.game_profile || has_overlay_customization {
            parts.push(format!("overlay_anchor={}", rule.overlay_anchor));
            parts.push(format!("overlay_offset_x={}", rule.overlay_offset_x));
            parts.push(format!("overlay_offset_y={}", rule.overlay_offset_y));
            parts.push(format!("overlay_scale={}", rule.overlay_scale_percent));
            parts.push(format!("overlay_monitor={}", rule.overlay_monitor));
            parts.push(format!("overlay_backend={}", rule.overlay_backend));
        }
        lines.push(parts.join(", "));
    }
    lines.join("\n")
}

pub(crate) fn split_inline_list(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|line| line.split(','))
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

pub(crate) fn add_compat_rule_unique(
    rules: &mut Vec<CompatRule>,
    process: &str,
    policy: CompatRulePolicy,
) -> bool {
    let process = normalized_process_name(process);
    if process.is_empty() {
        return false;
    }
    if rules
        .iter()
        .any(|rule| rule.process.eq_ignore_ascii_case(&process))
    {
        return false;
    }
    rules.push(CompatRule {
        enabled: true,
        process,
        policy,
        commit_transport: "global".to_string(),
        game_input_mode: "inherit".to_string(),
        game_chat: GameChatOptions::inherited(),
        game_profile: false,
        overlay_anchor: schema_default::OVERLAY_ANCHOR.to_string(),
        overlay_offset_x: 0,
        overlay_offset_y: 0,
        overlay_scale_percent: schema_default::OVERLAY_SCALE_PERCENT,
        overlay_monitor: schema_default::OVERLAY_MONITOR.to_string(),
        overlay_backend: schema_default::OVERLAY_BACKEND.to_string(),
    });
    true
}

pub(crate) fn upsert_compat_rule(
    rules: &mut Vec<CompatRule>,
    process: &str,
    enabled: bool,
    policy: CompatRulePolicy,
    commit_transport: &str,
    game_profile: bool,
) {
    let process = normalized_process_name(process);
    if process.is_empty() {
        return;
    }
    let commit_transport = normalize_commit_transport_value(commit_transport, "global");
    if let Some(rule) = rules
        .iter_mut()
        .find(|rule| rule.process.eq_ignore_ascii_case(&process))
    {
        rule.enabled = enabled;
        rule.policy = policy;
        rule.commit_transport = commit_transport;
        rule.game_profile = game_profile;
    } else {
        rules.push(CompatRule {
            enabled,
            process,
            policy,
            commit_transport,
            game_input_mode: "inherit".to_string(),
            game_chat: GameChatOptions::inherited(),
            game_profile,
            overlay_anchor: schema_default::OVERLAY_ANCHOR.to_string(),
            overlay_offset_x: 0,
            overlay_offset_y: 0,
            overlay_scale_percent: schema_default::OVERLAY_SCALE_PERCENT,
            overlay_monitor: schema_default::OVERLAY_MONITOR.to_string(),
            overlay_backend: schema_default::OVERLAY_BACKEND.to_string(),
        });
    }
}

pub(crate) fn normalized_process_name(process: &str) -> String {
    process.trim().trim_matches('"').trim().to_string()
}

pub(crate) fn remove_app_sections(config: &mut IniDoc) {
    let sections: Vec<String> = config
        .sections
        .keys()
        .filter(|name| name.starts_with("app:"))
        .cloned()
        .collect();
    for section in sections {
        config.remove_section(&section);
    }
}

pub(crate) fn apply_app_rules(config: &mut IniDoc, rules: &str) {
    for line in rules.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let mut parts = line
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty());
        let Some(app) = parts.next() else {
            continue;
        };
        let section = format!("app:{app}");
        for part in parts {
            let Some((key, value)) = part.split_once('=') else {
                continue;
            };
            match key.trim().to_ascii_lowercase().as_str() {
                "ascii" | "ascii_mode" => config.set(&section, "ascii_mode", value.trim()),
                "hide" | "hide_ui" => config.set(&section, "hide_ui", value.trim()),
                "inline" | "inline_preedit" => config.set(&section, "inline_preedit", value.trim()),
                "position" | "enhanced_position" => {
                    config.set(&section, "enhanced_position", value.trim())
                }
                "topmost" | "candidate_topmost" => {
                    config.set(&section, "candidate_topmost", value.trim())
                }
                "focus" | "focus_policy" => config.set(&section, "focus_policy", value.trim()),
                "commit" | "transport" | "commit_transport" => {
                    config.set(&section, "commit_transport", value.trim())
                }
                "game" | "profile" | "candidate_profile" | "game_profile" => {
                    config.set(&section, schema_key::GAME_PROFILE, value.trim())
                }
                "anchor" | "overlay_anchor" => {
                    config.set(&section, schema_key::OVERLAY_ANCHOR, value.trim())
                }
                "offset_x" | "overlay_offset_x" => {
                    config.set(&section, schema_key::OVERLAY_OFFSET_X, value.trim())
                }
                "offset_y" | "overlay_offset_y" => {
                    config.set(&section, schema_key::OVERLAY_OFFSET_Y, value.trim())
                }
                "scale" | "overlay_scale" => {
                    config.set(&section, schema_key::OVERLAY_SCALE, value.trim())
                }
                "monitor" | "overlay_monitor" => {
                    config.set(&section, schema_key::OVERLAY_MONITOR, value.trim())
                }
                "backend" | "overlay_backend" => {
                    config.set(&section, schema_key::OVERLAY_BACKEND, value.trim())
                }
                "game_enter_behavior"
                | "game_auto_uia"
                | "game_status_indicator"
                | "game_chat_open_key"
                | "game_chat_close_key"
                | "overlay_force_ui"
                | "game_tested_display_mode"
                | "game_tested_game_version"
                | "game_tested_input"
                | "game_tested_candidate"
                | "game_tested_commit"
                | "game_tested_exit" => config.set(&section, key.trim(), value.trim()),
                "game_input_mode" => config.set(
                    &section,
                    "game_input_mode",
                    normalize_game_input_mode(value, true),
                ),
                "enabled" | "policy" => config.set(&section, key.trim(), value.trim()),
                _ => {}
            }
        }
    }
}

pub(crate) fn parse_bool_value(value: &str, default: bool) -> bool {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" | "enabled" => true,
        "0" | "false" | "no" | "off" | "disabled" => false,
        _ => default,
    }
}

#[cfg(test)]
mod game_policy_tests {
    use super::*;
    #[test]
    fn game_chat_options_and_confirmed_results_round_trip() {
        let doc = parse_ini("[compatibility]\ngame_enter_behavior=close\ngame_auto_uia=0\ngame_status_indicator=1\n[app:mygame.exe]\ngame_profile=compact\ngame_enter_behavior=stay\ngame_auto_uia=inherit\ngame_status_indicator=0\ngame_chat_open_key=Enter\ngame_chat_close_key=Escape\noverlay_force_ui=1\ngame_tested_display_mode=borderless\ngame_tested_game_version=1.2\ngame_tested_input=1\ngame_tested_candidate=1\ngame_tested_commit=1\ngame_tested_exit=1\n");
        let model = model_from_config(&doc);
        assert_eq!(model.game_chat.enter_behavior, "close");
        assert_eq!(model.game_chat.auto_uia, "0");
        let chat = &model.compat_rules[0].game_chat;
        assert_eq!(chat.enter_behavior, "stay");
        assert_eq!(chat.auto_uia, "inherit");
        assert_eq!(chat.status_indicator, "0");
        assert_eq!(chat.open_key, "Enter");
        assert!(
            chat.force_ui
                && chat.tested_input
                && chat.tested_candidate
                && chat.tested_commit
                && chat.tested_exit
        );
        let rendered = stabilized_rendered_config(&doc, &model);
        let saved = model_from_config(&parse_ini(&rendered));
        assert_eq!(saved.game_chat, model.game_chat);
        assert_eq!(saved.compat_rules[0].game_chat, *chat);
        assert_eq!(
            saved.compat_rules[0].game_chat.tested_display_mode,
            "borderless"
        );
        let mut changed = saved.compat_rules[0].game_chat.clone();
        changed.reset_test_results();
        assert!(
            !changed.tested_input
                && !changed.tested_candidate
                && !changed.tested_commit
                && !changed.tested_exit
        );
    }

    #[test]
    fn game_chat_defaults_do_not_forge_compatibility_results() {
        let model = model_from_config(&parse_ini("[app:unknown.exe]\ngame_profile=compact\n"));
        let chat = &model.compat_rules[0].game_chat;
        assert_eq!(chat, &GameChatOptions::inherited());
        assert_eq!(normalize_game_enter_behavior("bad", false), "auto");
        assert_eq!(normalize_game_option_bool("bad", true), "inherit");
        assert!(!chat.force_ui && !chat.tested_input && !chat.tested_commit);
    }

    #[test]
    fn confirmed_results_survive_confirmation_but_expire_when_policy_changes() {
        let before = model_from_config(&parse_ini(
            "[app:game.exe]\ngame_profile=compact\ngame_tested_display_mode=borderless\n",
        ));
        let mut confirmed = before.clone();
        confirmed.compat_rules[0].game_chat.tested_input = true;
        confirmed.compat_rules[0].game_chat.tested_candidate = true;
        invalidate_changed_game_test_results(&before, &mut confirmed);
        assert!(confirmed.compat_rules[0].game_chat.tested_input);
        let saved = confirmed.clone();
        confirmed.compat_rules[0].overlay_backend = "external".into();
        invalidate_changed_game_test_results(&saved, &mut confirmed);
        assert!(!confirmed.compat_rules[0].game_chat.tested_input);
        assert!(!confirmed.compat_rules[0].game_chat.tested_candidate);
        let mut changed_global = saved.clone();
        changed_global.game_input_mode = "auto_text".into();
        invalidate_changed_game_test_results(&saved, &mut changed_global);
        assert!(!changed_global.compat_rules[0].game_chat.tested_input);
        assert_eq!(
            changed_global.compat_rules[0].game_chat.tested_display_mode,
            "borderless"
        );
    }

    #[test]
    fn game_modes_survive_settings_save_and_reload() {
        for mode in ["manual", "passthrough", "chinese", "auto_text"] {
            let input = format!("[compatibility]\ngame_input_mode={mode}\n[app:mygame.exe]\ngame_profile=compact\ngame_input_mode={mode}\ncommit_transport=clipboard_paste\noverlay_anchor=bottom_left\n");
            let mut config = parse_ini(&input);
            let mut model = model_from_config(&config);
            assert_eq!(model.game_input_mode, mode);
            let rule = model
                .compat_rules
                .iter()
                .find(|r| r.process == "mygame.exe")
                .unwrap();
            assert_eq!(rule.game_input_mode, mode);
            assert_eq!(rule.commit_transport, "clipboard_paste");
            sync_compat_rules_to_legacy_fields(&mut model);
            apply_model_to_config(&mut config, &model);
            let reloaded =
                model_from_config(&parse_ini(&rendered_config_for_model(&config, &model)));
            let saved = reloaded
                .compat_rules
                .iter()
                .find(|r| r.process == "mygame.exe")
                .unwrap();
            assert_eq!(saved.game_input_mode, mode);
            assert_eq!(saved.commit_transport, "clipboard_paste");
            assert_eq!(saved.overlay_anchor, "bottom_left");
        }
    }
    #[test]
    fn defaults_and_inherited_games_remain_distinct() {
        assert_eq!(model_from_config(&parse_ini("")).game_input_mode, "manual");
        assert_eq!(normalize_game_input_mode("invalid", true), "inherit");
        assert_eq!(normalize_game_input_mode("invalid", false), "manual");
        let model = model_from_config(&parse_ini("[app:custom.exe]\ngame_profile=compact\n"));
        assert_eq!(model.compat_rules[0].game_input_mode, "inherit");
    }
}

#[cfg(test)]
mod shared_config_tests {
    use super::*;
    #[test]
    fn defaults_preserve_comments_extensions_and_migration_version() {
        let doc = parse_ini(
            "; personal comment\n[style]\ncandidate_density=compact\n[extension]\nfuture=keep\n",
        );
        assert_eq!(read_config_version(&doc), 0);
        assert_eq!(doc.get("style", "candidate_density"), Some("compact"));
        assert_eq!(doc.get("compatibility", "game_input_mode"), Some("manual"));
        let text = doc.render();
        assert!(text.contains("; personal comment"));
        assert!(text.contains("future=keep"));
        assert_eq!(parse_ini(&text).get("extension", "future"), Some("keep"));
    }
}

#[cfg(test)]
mod halfwidth_tests {
    use super::*;

    #[test]
    fn chinese_halfwidth_defaults_off_and_survives_save_reload() {
        assert!(!model_from_config(&parse_ini("")).chinese_halfwidth);
        for enabled in [true, false] {
            let mut doc = parse_ini(
                "[input]\ndefault_full_shape=1\nsymbol_fullwidth=1\nnumber_fullwidth=1\n",
            );
            let mut model = model_from_config(&doc);
            model.chinese_halfwidth = enabled;
            apply_model_to_config(&mut doc, &model);
            let saved = rendered_config_for_model(&doc, &model);
            let reloaded = model_from_config(&parse_ini(&saved));
            assert_eq!(reloaded.chinese_halfwidth, enabled);
            assert!(
                reloaded.default_full_shape
                    && reloaded.symbol_fullwidth
                    && reloaded.number_fullwidth
            );
        }
    }
}
