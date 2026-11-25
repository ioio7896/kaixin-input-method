#![allow(
    clippy::empty_line_after_doc_comments,
    clippy::if_same_then_else,
    clippy::needless_range_loop,
    clippy::ptr_arg,
    clippy::type_complexity
)]

pub mod clipboard_store;
#[cfg(windows)]
use kaixin_platform::win_handle;
use kaixin_platform::{app_paths, encrypted_journal, runtime_log};
