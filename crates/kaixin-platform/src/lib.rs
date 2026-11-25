#![allow(
    clippy::empty_line_after_doc_comments,
    clippy::if_same_then_else,
    clippy::needless_range_loop,
    clippy::ptr_arg,
    clippy::type_complexity
)]

pub mod app_paths;
pub mod encrypted_journal;
pub mod rapidocr_paths;
pub mod runtime_log;
pub mod tool_prefs;
#[cfg(windows)]
pub mod win_handle;
pub mod win_paste;
#[cfg(windows)]
pub mod win_single_instance;
pub mod win_target;
pub mod windows_security;
pub use kaixin_common::shared_rules;
#[cfg(windows)]
pub mod storage_client;
