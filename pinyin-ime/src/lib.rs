#![allow(
    clippy::empty_line_after_doc_comments,
    clippy::if_same_then_else,
    clippy::needless_range_loop,
    clippy::ptr_arg,
    clippy::type_complexity
)]

//! Shared library for the GUI, settings app, background engine, and build tools.
pub const ENGINE_PANIC_RC: i32 = -100;
pub mod app_paths;
pub mod candidate_prefs;
pub mod clipboard_store;
mod compiled_data;
pub mod config_schema;
pub mod core;
pub mod correction_prefs;
pub mod custom_shortcuts;
pub mod dict;
pub mod dxgi_capture;
pub mod engine;
pub mod english_words;
pub mod external_translation;
pub mod fuzzy_prefs;
pub mod handwrite_lookup;
#[cfg(windows)]
pub mod ipc_service;
pub mod lexicon_prefs;
pub mod lm;
pub mod rapidocr_paths;
pub mod rerank_prefs;
pub mod runtime_config;
pub mod runtime_log;
pub mod screenshot_region_selector;
pub mod screenshot_store;
pub mod segment;
pub mod shared_rules;
pub mod text_encoding;
pub mod text_norm;
pub mod thuocl;
pub mod tool_prefs;
pub mod traditional;
pub mod ui_theme;
pub mod user_dict;
pub mod user_dict_io;
pub mod user_hotword_prefs;
pub mod v_mode;
pub mod v_tools;
#[cfg(windows)]
pub mod win_handle;
pub mod win_paste;
pub mod win_single_instance;
pub mod windows_graphics_capture;
pub mod windows_security;
