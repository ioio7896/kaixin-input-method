#![allow(
    clippy::empty_line_after_doc_comments,
    clippy::if_same_then_else,
    clippy::needless_range_loop,
    clippy::ptr_arg,
    clippy::type_complexity
)]

pub mod compiled_data;
pub mod dict;
pub mod engine;
pub mod lm;
pub mod model_resources;
pub mod segment;
pub mod text_norm;
pub mod traditional;
