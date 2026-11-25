#![allow(
    clippy::empty_line_after_doc_comments,
    clippy::if_same_then_else,
    clippy::needless_range_loop,
    clippy::ptr_arg,
    clippy::type_complexity
)]

pub mod dxgi_capture;
pub mod screenshot_capture;
pub mod screenshot_region_selector;
pub mod screenshot_store;
pub mod screenshot_trace;
pub mod ui_theme;
pub mod windows_graphics_capture;
use kaixin_platform::{runtime_log, tool_prefs};
