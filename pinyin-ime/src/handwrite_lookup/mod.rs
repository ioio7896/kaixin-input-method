#![allow(
    clippy::empty_line_after_outer_attr,
    clippy::legacy_numeric_constants,
    clippy::needless_late_init,
    clippy::needless_range_loop,
    clippy::ptr_arg,
    clippy::same_item_push,
    clippy::too_many_arguments,
    dead_code,
    mismatched_lifetime_syntaxes
)]

//! Offline handwriting lookup based on HanziLookup.
//!
//! The matcher and embedded stroke data are adapted from the open-source
//! `hanzi_lookup` project so the handwriting panel can stay fully local.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;

mod analyzed_character;
mod cubic_curve_2d;
mod entities;
mod match_collector;
mod matcher;

use match_collector::MatchCollector;
use matcher::Matcher;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: u8,
    pub y: u8,
}

#[derive(Debug, Clone)]
pub struct Stroke {
    pub points: Vec<Point>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq)]
pub struct Match {
    pub hanzi: char,
    pub score: f32,
}

thread_local!(static MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new()));

pub fn match_typed(strokes: &[Stroke], limit: usize) -> Vec<Match> {
    if limit == 0 || strokes.is_empty() {
        return Vec::new();
    }
    let mut res = Vec::with_capacity(limit);
    let mut collector = MatchCollector::new(&mut res, limit);
    let owned = strokes.to_vec();
    MATCHER.with(|matcher| {
        matcher.borrow_mut().lookup(&owned, &mut collector);
    });
    res
}
