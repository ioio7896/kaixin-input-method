#![allow(
    clippy::empty_line_after_doc_comments,
    clippy::if_same_then_else,
    clippy::needless_range_loop,
    clippy::ptr_arg,
    clippy::type_complexity
)]

//! Shared library for the GUI, settings app, background engine, and build tools.
pub const ENGINE_PANIC_RC: i32 = -100;
pub mod candidate_prefs;
mod cold_lexicon;
pub mod config_schema;
pub mod core;
pub mod correction_prefs;
pub mod custom_shortcuts;
pub mod engine_health;
pub mod english_words;
pub mod eval_profile;
pub mod fuzzy_prefs;
pub mod game_keyboard_policy;
pub mod handwrite_lookup;
#[cfg(windows)]
pub mod ipc_service;
pub mod lexicon_prefs;
pub mod rerank_prefs;
pub mod runtime_config;
pub mod shared_rules;
pub mod text_encoding;
pub mod thuocl;
pub mod user_dict;
pub mod user_dict_io;
pub mod user_hotword_prefs;
pub mod v_mode;
pub mod v_tools;
#[cfg(feature = "desktop")]
pub use kaixin_capture::{
    dxgi_capture, screenshot_capture, screenshot_region_selector, screenshot_store,
    screenshot_trace, ui_theme, windows_graphics_capture,
};
pub use kaixin_clipboard::clipboard_store;
use kaixin_pinyin::compiled_data;
pub use kaixin_pinyin::{dict, engine, lm, model_resources, segment, text_norm, traditional};
use kaixin_platform::encrypted_journal;
pub use kaixin_platform::{
    app_paths, rapidocr_paths, runtime_log, tool_prefs, win_paste, win_target, windows_security,
};
#[cfg(windows)]
pub use kaixin_platform::{win_handle, win_single_instance};
pub use kaixin_translate::external_translation;
