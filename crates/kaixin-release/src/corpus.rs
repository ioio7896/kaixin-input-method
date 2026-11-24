use encoding_rs::{GB18030, UTF_16BE, UTF_16LE};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::{is_han, read_utf8, text_files, Result};

type Weights = HashMap<usize, Vec<u64>>;

fn layer(path: &Path) -> &str {
    let has = |name| path.iter().any(|part| part.eq_ignore_ascii_case(name));
    if has("zh-ext") {
        return "ext";
    }
    if let Some(name) = ["core", "base", "ext", "large", "en"]
        .into_iter()
        .find(|name| has(name))
    {
        return name;
    }
    if !has("zh") {
        return "";
    }
    let name = path
        .file_name()
        .expect("text file name")
        .to_string_lossy()
        .to_ascii_lowercase();
    match name.as_str() {
        "kaixin_explicit.txt"
        | "kaixin_polyphone.txt"
        | "kaixin_pronunciation_aliases.txt"
        | "life_hot_3char_curated.txt"
        | "life_hot_4char_curated.txt" => "core",
        _ if name.contains("_tail_") || name.starts_with("large_") => "large",
        _ => "base",
    }
}

fn include(path: &Path, retired_files: &[String], retired_tags: &[String]) -> bool {
    let name = path
        .file_name()
        .expect("text file name")
        .to_string_lossy()
        .to_ascii_lowercase();
    let relative = path
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if path.iter().any(|part| part.eq_ignore_ascii_case("en"))
        || name.starts_with("readme")
        || name.starts_with("professional_")
        || name.starts_with("hangzhou_")
        || name.ends_with("_纯名单.txt")
        || retired_files
            .iter()
            .any(|retired| relative == retired.to_ascii_lowercase())
    {
        return false;
    }
    let stem = name.strip_suffix(".txt").expect("text extension");
    let tag = stem
        .strip_prefix("thuocl_")
        .or_else(|| stem.split_once("__thuocl_").map(|(_, tag)| tag))
        .unwrap_or(stem);
    layer(path) != "ext" || !retired_tags.iter().any(|retired| retired == tag)
}

fn read_lexicon(path: &Path) -> Result<String> {
    let data = fs::read(path)?;
    let head = &data[..data.len().min(262_144)];
    let seems_utf16 = head.len() >= 4
        && head.len() % 2 == 0
        && head
            .iter()
            .skip(1)
            .step_by(2)
            .filter(|&&byte| byte == 0)
            .count()
            * 3
            >= head.len() / 2;
    if let Some(data) = data.strip_prefix(b"\xef\xbb\xbf") {
        return Ok(String::from_utf8_lossy(data).into_owned());
    }
    let utf16 = if let Some(data) = data.strip_prefix(b"\xff\xfe") {
        Some((data, UTF_16LE))
    } else if let Some(data) = data.strip_prefix(b"\xfe\xff") {
        Some((data, UTF_16BE))
    } else {
        seems_utf16.then_some((data.as_slice(), UTF_16LE))
    };
    if let Some((bytes, encoding)) = utf16 {
        return Ok(encoding.decode_without_bom_handling(bytes).0.into_owned());
    }
    if std::str::from_utf8(head).is_err() && !GB18030.decode_without_bom_handling(head).1 {
        return Ok(GB18030.decode_without_bom_handling(&data).0.into_owned());
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

fn rows(text: &str) -> impl Iterator<Item = (&str, u64)> {
    text.split(['\r', '\n']).filter_map(|line| {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.starts_with('#') {
            return None;
        }
        let mut fields = line.split('\t');
        let term = fields.next()?.trim();
        if !term.chars().any(is_han) {
            return None;
        }
        let weight = fields
            .rev()
            .find_map(|value| value.trim().parse::<i64>().ok())
            .unwrap_or(1);
        Some((term, weight.max(1) as u64))
    })
}

fn length(term: &str) -> usize {
    term.chars().filter(|ch| !ch.is_whitespace()).count()
}

fn normalized(term: &str, weight: u64, source: &Weights, reference: &Weights) -> u64 {
    let length = length(term);
    let Some(values) = source.get(&length).filter(|values| !values.is_empty()) else {
        return weight;
    };
    let Some(reference) = reference.get(&length).filter(|values| !values.is_empty()) else {
        return weight;
    };
    let start = values.partition_point(|&value| value < weight);
    let end = values.partition_point(|&value| value <= weight);
    let rank = (start + (end - start).saturating_sub(1) / 2).min(values.len() - 1);
    let index = if values.len() < 2 {
        0
    } else {
        rank * (reference.len() - 1) / (values.len() - 1)
    };
    reference[index].max(1)
}

pub fn build(repo: &Path) -> Result<()> {
    let lexicon = repo.join("lexicon");
    let retired_files: Vec<String> =
        serde_json::from_str(&read_utf8(&repo.join("shared/retired_lexicon_files.json"))?)?;
    let retired_tags: Vec<String> = serde_json::from_str(&read_utf8(
        &repo.join("shared/retired_optional_lexicons.json"),
    )?)?;
    let mut reference = Weights::new();
    let mut sources = Vec::new();
    let mut use_fixed_reference = false;
    for path in text_files(&lexicon)? {
        let relative = path.strip_prefix(&lexicon)?;
        if !include(relative, &retired_files, &retired_tags) {
            continue;
        }
        let text = read_lexicon(&path)?;
        let name = relative
            .file_name()
            .expect("text file name")
            .to_string_lossy()
            .to_ascii_lowercase();
        let layer = layer(relative);
        use_fixed_reference |= name == "life_common_3char.txt";
        let unscaled = matches!(
            name.as_str(),
            "people_names.txt" | "hangzhou_local.txt" | "daily_communication.txt"
        ) || (layer == "core"
            && matches!(
                name.as_str(),
                "kaixin_polyphone.txt" | "kaixin_pronunciation_aliases.txt"
            ));
        let mut distribution = Weights::new();
        let weights = if layer == "base" {
            Some(&mut reference)
        } else if unscaled {
            None
        } else {
            Some(&mut distribution)
        };
        if let Some(weights) = weights {
            for (term, weight) in rows(&text) {
                weights.entry(length(term)).or_default().push(weight);
            }
        }
        distribution
            .values_mut()
            .for_each(|weights| weights.sort_unstable());
        sources.push((text, distribution));
    }
    if use_fixed_reference {
        let fixed: Weights = serde_json::from_str(&read_utf8(
            &repo.join("shared/lexicon_frequency_reference.json"),
        )?)?;
        reference.extend(fixed);
    }
    reference
        .values_mut()
        .for_each(|weights| weights.sort_unstable());

    let mut terms: HashMap<&str, u64> = HashMap::new();
    for (text, distribution) in &sources {
        for (term, weight) in rows(text) {
            let weight = normalized(term, weight, distribution, &reference);
            terms
                .entry(term)
                .and_modify(|old| *old = (*old).max(weight))
                .or_insert(weight);
        }
    }
    if terms.is_empty() {
        return Err("no Chinese corpus terms found".into());
    }
    let mut ordered: Vec<_> = terms.into_iter().collect();
    ordered.sort_unstable_by(|(term_a, weight_a), (term_b, weight_b)| {
        weight_b.cmp(weight_a).then_with(|| term_a.cmp(term_b))
    });
    let mut corpus = Vec::new();
    for &(term, weight) in &ordered {
        let repeats =
            [100, 2_500, 5_000, 7_500].partition_point(|&threshold| threshold <= weight) + 1;
        for _ in 0..repeats {
            writeln!(&mut corpus, "{term}")?;
        }
    }
    let output = repo.join("pinyin-ime/data/corpus.txt");
    if fs::read(&output).ok().as_deref() != Some(corpus.as_slice()) {
        let pending = output.with_extension("txt.tmp");
        fs::write(&pending, &corpus)?;
        fs::rename(&pending, &output)?;
    }
    println!("Corpus: {} terms, {} bytes", ordered.len(), corpus.len());
    Ok(())
}
