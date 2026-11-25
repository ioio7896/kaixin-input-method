use crate::model_resources::{open_resource, MappedResource, S2T_FILE};
use std::collections::HashMap;
use std::sync::OnceLock;

fn s2t_map() -> &'static HashMap<char, &'static str> {
    static DATA: OnceLock<Option<MappedResource>> = OnceLock::new();
    static MAP: OnceLock<HashMap<char, &'static str>> = OnceLock::new();
    MAP.get_or_init(|| {
        DATA.get_or_init(|| open_resource(S2T_FILE).ok())
            .as_ref()
            .map(|mapped| &mapped[..])
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or_default()
            .lines()
            .filter_map(|line| {
                let (source, target) = line.split_once('\t')?;
                let mut chars = source.chars();
                let first = chars.next()?;
                (chars.next().is_none() && !target.is_empty()).then_some((first, target))
            })
            .collect()
    })
}

pub fn to_traditional(text: &str) -> String {
    let map = s2t_map();
    let mut out = String::with_capacity(text.len());
    let mut changed = false;
    for ch in text.chars() {
        if let Some(converted) = map.get(&ch) {
            out.push_str(converted);
            changed = true;
        } else {
            out.push(ch);
        }
    }
    if changed {
        out
    } else {
        text.to_string()
    }
}
