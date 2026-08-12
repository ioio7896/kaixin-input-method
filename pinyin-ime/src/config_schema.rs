pub mod section {
    pub const STYLE: &str = "style";
    pub const COMPATIBILITY: &str = "compatibility";
    pub const PRIVACY: &str = "privacy";
    pub const ENGINE: &str = "engine";
}

pub mod key {
    pub const THEME: &str = "theme";
    pub const CANDIDATE_MATERIAL: &str = "candidate_material";
    pub const CANDIDATE_DENSITY: &str = "candidate_density";
    pub const CANDIDATE_LAYOUT_VARIANT: &str = "candidate_layout_variant";
    pub const CANDIDATE_VERTICAL_LAYOUT_VARIANT: &str = "candidate_vertical_layout_variant";
    pub const CANDIDATE_HORIZONTAL_LAYOUT_VARIANT: &str = "candidate_horizontal_layout_variant";
    pub const CANDIDATE_REDUCE_MOTION: &str = "candidate_reduce_motion";
    pub const HIGHLIGHT_TYPO_CANDIDATES: &str = "highlight_typo_candidates";
    pub const SHOW_CANDIDATE_SOURCE: &str = "show_candidate_source";
    pub const LEARNING_SENSITIVITY: &str = "learning_sensitivity";
    pub const USER_HOTWORD_BOOST: &str = "user_hotword_boost";

    pub const FULLSCREEN_DETECTION: &str = "fullscreen_detection";
    pub const FULLSCREEN_POLICY: &str = "fullscreen_policy";
    pub const COMMIT_TRANSPORT: &str = "commit_transport";
    pub const GAME_PROFILE: &str = "game_profile";
    pub const OVERLAY_ANCHOR: &str = "overlay_anchor";
    pub const OVERLAY_OFFSET_X: &str = "overlay_offset_x";
    pub const OVERLAY_OFFSET_Y: &str = "overlay_offset_y";
    pub const OVERLAY_SCALE: &str = "overlay_scale";
    pub const OVERLAY_MONITOR: &str = "overlay_monitor";
    pub const OVERLAY_BACKEND: &str = "overlay_backend";
    pub const BUILTIN_GAME_LIST: &str = "builtin_game_list";
    pub const AUTO_SUGGEST_APP_OPTIONS: &str = "auto_suggest_app_options";
    pub const GAME_PROCESSES: &str = "game_processes";

    pub const NEVER_LEARN_PROCESSES: &str = "never_learn_processes";
    pub const NEVER_CLIPBOARD_PROCESSES: &str = "never_clipboard_processes";
    pub const NEVER_CANDIDATE_PROCESSES: &str = "never_candidate_processes";
    pub const PRIVACY_ENABLED: &str = "enabled";

    pub const PREFIX_CACHE_CAPACITY: &str = "prefix_cache_capacity";
    pub const FINAL_LOOKUP_CACHE_CAPACITY: &str = "final_lookup_cache_capacity";
    pub const SHORT_LOOKUP_CACHE_CAPACITY: &str = "short_lookup_cache_capacity";
    pub const LONG_LOOKUP_SOFT_BUDGET_MS: &str = "long_lookup_soft_budget_ms";
    pub const LONG_LOOKUP_MIN_FIRST_BATCH_CANDIDATES: &str =
        "long_lookup_min_first_batch_candidates";
}

pub mod default {
    pub const THEME: &str = "auto";
    pub const CANDIDATE_MATERIAL: &str = "auto";
    pub const CANDIDATE_DENSITY: &str = "standard";
    pub const CANDIDATE_LAYOUT_VARIANT: &str = "compact";
    pub const CANDIDATE_HORIZONTAL_LAYOUT_VARIANT: &str = "classic";
    pub const FULLSCREEN_POLICY: &str = "show_ui";
    pub const COMMIT_TRANSPORT: &str = "tsf";
    pub const OVERLAY_ANCHOR: &str = "auto";
    pub const OVERLAY_SCALE_PERCENT: usize = 100;
    pub const OVERLAY_MONITOR: &str = "auto";
    pub const OVERLAY_BACKEND: &str = "auto";
    pub const LEARNING_SENSITIVITY: &str = "standard";
    pub const USER_HOTWORD_BOOST: &str = "standard";
    // Performance-oriented defaults: keep common incremental prefixes and
    // completed readings hot while allowing the first page to return quickly.
    pub const PREFIX_CACHE_CAPACITY: usize = 384;
    pub const FINAL_LOOKUP_CACHE_CAPACITY: usize = 128;
    pub const SHORT_LOOKUP_CACHE_CAPACITY: usize = 192;
    pub const LONG_LOOKUP_SOFT_BUDGET_MS: usize = 4;
    pub const LONG_LOOKUP_MIN_FIRST_BATCH_CANDIDATES: usize = 6;
}

pub mod options {
    pub const THEMES: &[&str] = &["auto", "light", "dark", "high_contrast"];
    pub const CANDIDATE_MATERIALS: &[&str] = &["auto", "solid", "gradient", "mist"];
    pub const CANDIDATE_DENSITIES: &[&str] = &["compact", "standard", "comfortable"];
    pub const CANDIDATE_LAYOUTS: &[&str] = &["classic", "compact", "card"];
    pub const FULLSCREEN_POLICIES: &[&str] = &["show_ui", "ascii", "hide_ui", "off"];
    pub const COMMIT_TRANSPORTS: &[&str] = &["auto", "tsf", "clipboard_paste", "unicode_sendinput"];
    pub const OVERLAY_ANCHORS: &[&str] = &[
        "auto",
        "caret",
        "top_left",
        "top_center",
        "top_right",
        "bottom_left",
        "bottom_center",
        "bottom_right",
    ];
    pub const OVERLAY_BACKENDS: &[&str] = &["auto", "in_process", "external"];
    pub const LEARNING_SENSITIVITY: &[&str] = &["conservative", "standard", "aggressive"];
    pub const USER_HOTWORD_BOOST: &[&str] = &["conservative", "standard", "strong", "aggressive"];
}
