use kaixin_pinyin::model_resources::{
    ALIASES_FILE, BIGRAM_ENTRY_BYTES, EXCLUSIONS_FILE, MODEL_FILE, MODEL_MAGIC,
    MODEL_SCHEMA_VERSION, RESOURCE_DIR, RESOURCE_MANIFEST, S2T_FILE, SYMBOL_FILE,
};
use kaixin_pinyin::{dict, lm::Lm};
use rusqlite::{Connection, OpenFlags};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

fn decode_text_bytes(bytes: &[u8]) -> Result<String, String> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).map_err(|err| format!("invalid utf-8: {err}"));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return decode_utf16_bytes(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return decode_utf16_bytes(rest, false);
    }
    if looks_like_utf16(bytes, true) {
        if let Ok(text) = decode_utf16_bytes(bytes, true) {
            return Ok(text);
        }
    }
    if looks_like_utf16(bytes, false) {
        if let Ok(text) = decode_utf16_bytes(bytes, false) {
            return Ok(text);
        }
    }
    if let Ok(text) = String::from_utf8(bytes.to_vec()) {
        return Ok(text);
    }
    Err("text is neither valid UTF-8 nor a recognizable UTF-16 variant".to_string())
}

fn looks_like_utf16(bytes: &[u8], little_endian: bool) -> bool {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return false;
    }

    let nul_index = if little_endian { 1 } else { 0 };
    let other_index = 1 - nul_index;
    let mut nul_count = 0usize;
    let mut other_nul_count = 0usize;
    let mut pair_count = 0usize;

    for pair in bytes.as_chunks::<2>().0 {
        pair_count += 1;
        if pair[nul_index] == 0 {
            nul_count += 1;
        }
        if pair[other_index] == 0 {
            other_nul_count += 1;
        }
    }

    nul_count > 0 && nul_count * 3 >= pair_count && nul_count >= other_nul_count
}

fn decode_utf16_bytes(bytes: &[u8], little_endian: bool) -> Result<String, String> {
    if !bytes.len().is_multiple_of(2) {
        return Err("utf-16 input has an odd byte length".to_string());
    }

    let units = bytes.as_chunks::<2>().0.iter().map(|pair| {
        if little_endian {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    });

    let mut out = String::new();
    for ch in std::char::decode_utf16(units) {
        match ch {
            Ok(ch) => out.push(ch),
            Err(err) => return Err(format!("invalid utf-16: {err}")),
        }
    }

    Ok(out.strip_prefix('\u{FEFF}').unwrap_or(&out).to_string())
}

fn write_u32<W: Write>(writer: &mut W, value: u32) {
    writer.write_all(&value.to_le_bytes()).expect("write u32");
}

fn write_count<W: Write>(writer: &mut W, count: usize) {
    let value = u32::try_from(count).expect("count exceeds u32");
    write_u32(writer, value);
}

fn write_string<W: Write>(writer: &mut W, value: &str) {
    let bytes = value.as_bytes();
    write_count(writer, bytes.len());
    writer.write_all(bytes).expect("write string");
}

fn write_char<W: Write>(writer: &mut W, value: char) {
    write_u32(writer, value as u32);
}

fn build_lm_counts(
    corpus: &str,
    vocab: &HashSet<char>,
) -> (HashMap<char, usize>, HashMap<(char, char), usize>) {
    let mut unigram: HashMap<char, usize> = HashMap::new();
    let mut bigram: HashMap<(char, char), usize> = HashMap::new();

    for line in corpus.lines() {
        let chars: Vec<char> = line.chars().filter(|ch| vocab.contains(ch)).collect();
        for &ch in &chars {
            *unigram.entry(ch).or_insert(0) += 1;
        }
        for window in chars.windows(2) {
            *bigram.entry((window[0], window[1])).or_insert(0) += 1;
        }
    }

    (unigram, bigram)
}

fn load_syllables(text: &str) -> Vec<String> {
    let mut values: Vec<String> = text
        .lines()
        .map(|line| line.trim().to_ascii_lowercase())
        .filter(|line| !line.is_empty())
        .collect();
    values.sort();
    values.dedup();
    values
}

fn write_compiled_model(
    out_dir: &Path,
    chars: &str,
    corpus: &str,
    syllables: &str,
    aliases: &str,
    exclusions: &str,
) -> Result<(), String> {
    let dict = dict::build_model_dict(chars, aliases, exclusions);
    let vocab: HashSet<char> = dict.values().flatten().copied().collect();
    let syllable_list = load_syllables(syllables);
    let (unigram, bigram) = build_lm_counts(corpus, &vocab);
    let char_count = chars.chars().filter(|ch| !ch.is_whitespace()).count();

    let path = out_dir.join(MODEL_FILE);
    let file = fs::File::create(path).map_err(|e| format!("create compiled model: {e}"))?;
    let mut writer = BufWriter::new(file);

    writer
        .write_all(MODEL_MAGIC)
        .map_err(|e| format!("write model magic: {e}"))?;
    write_u32(&mut writer, MODEL_SCHEMA_VERSION);
    write_count(&mut writer, char_count);

    let mut dict_entries: Vec<_> = dict.into_iter().collect();
    dict_entries.sort_by(|a, b| a.0.cmp(&b.0));
    write_count(&mut writer, dict_entries.len());
    for (key, chars) in dict_entries {
        write_string(&mut writer, &key);
        write_count(&mut writer, chars.len());
        for ch in chars {
            write_char(&mut writer, ch);
        }
    }

    write_count(&mut writer, syllable_list.len());
    for syllable in syllable_list {
        write_string(&mut writer, &syllable);
    }

    let mut unigram_entries: Vec<_> = unigram.into_iter().collect();
    unigram_entries.sort_by_key(|entry| entry.0);
    write_count(&mut writer, unigram_entries.len());
    for &(ch, count) in &unigram_entries {
        write_char(&mut writer, ch);
        write_count(&mut writer, count);
    }

    let smoothing = Lm::smoothing_mass(vocab.len());
    let counts: HashMap<_, _> = unigram_entries.into_iter().collect();
    let mut bigram_entries: Vec<_> = bigram.into_iter().collect();
    bigram_entries.sort_unstable_by_key(|entry| entry.0);
    write_count(&mut writer, bigram_entries.len());
    for ((left, right), count) in bigram_entries {
        let predecessor_count = counts.get(&left).copied().unwrap_or(0);
        let score = Lm::observed_bigram_logprob(count, predecessor_count, smoothing);
        let mut row = [0_u8; BIGRAM_ENTRY_BYTES];
        row[..4].copy_from_slice(&(left as u32).to_le_bytes());
        row[4..8].copy_from_slice(&(right as u32).to_le_bytes());
        row[8..].copy_from_slice(&score.to_le_bytes());
        writer.write_all(&row).map_err(|err| err.to_string())?;
    }

    writer
        .flush()
        .map_err(|e| format!("flush compiled model: {e}"))?;
    Ok(())
}

fn export_s2t(database: &Path, output: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let source = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut statement =
        source.prepare("SELECT simplified, traditional FROM s2t_chars ORDER BY sort_order")?;
    let mut rows = statement.query([])?;
    let mut output = BufWriter::new(fs::File::create(output)?);
    while let Some(row) = rows.next()? {
        let simplified: String = row.get(0)?;
        let traditional: String = row.get(1)?;
        if simplified.chars().count() == 1 && !traditional.is_empty() {
            if simplified.contains(['\t', '\n', '\r']) || traditional.contains(['\t', '\n', '\r']) {
                return Err("invalid control character in traditional conversion table".into());
            }
            writeln!(output, "{simplified}\t{traditional}")?;
        }
    }
    output.flush()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1);
    let repo = args
        .next()
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?);
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("pinyin-ime/target").join(RESOURCE_DIR));
    if args.next().is_some() {
        return Err("usage: bake_model [repository] [model-directory]".into());
    }
    let data = repo.join("pinyin-ime/data");
    let pronunciations = repo.join("data_sources/kaixin");
    let chars = decode_text_bytes(&fs::read(data.join("pinyin_supported_chars.txt"))?)?;
    let corpus = decode_text_bytes(&fs::read(data.join("corpus.txt"))?)?;
    let syllables = decode_text_bytes(&fs::read(data.join("syllables.txt"))?)?;
    let aliases = fs::read_to_string(pronunciations.join(ALIASES_FILE))?;
    let exclusions = fs::read_to_string(pronunciations.join(EXCLUSIONS_FILE))?;

    let pending = output.with_extension(format!("pending-{}", std::process::id()));
    fs::create_dir_all(&pending)?;
    write_compiled_model(&pending, &chars, &corpus, &syllables, &aliases, &exclusions)?;
    export_s2t(&data.join("s2t_chars.sqlite"), &pending.join(S2T_FILE))?;
    fs::copy(data.join(SYMBOL_FILE), pending.join(SYMBOL_FILE))?;
    fs::write(pending.join(ALIASES_FILE), aliases)?;
    fs::write(pending.join(EXCLUSIONS_FILE), exclusions)?;
    let files = [
        MODEL_FILE,
        S2T_FILE,
        SYMBOL_FILE,
        ALIASES_FILE,
        EXCLUSIONS_FILE,
    ];
    fs::create_dir_all(&output)?;
    let mut manifest = String::new();
    for name in files {
        let path = pending.join(name);
        let digest = format!("{:x}", Sha256::digest(fs::read(&path)?));
        let published = output.join(format!("{digest}-{name}"));
        let is_current =
            published.is_file() && format!("{:x}", Sha256::digest(fs::read(&published)?)) == digest;
        if is_current {
            fs::remove_file(path)?;
        } else {
            fs::rename(path, published)?;
        }
        manifest.push_str(&format!("{digest}\t{name}\n"));
    }
    // A single rename publishes the set after all its files are available.
    fs::write(pending.join(RESOURCE_MANIFEST), manifest)?;
    fs::rename(
        pending.join(RESOURCE_MANIFEST),
        output.join(RESOURCE_MANIFEST),
    )?;
    fs::remove_dir(pending)?;
    println!("Generated model resources in {}", output.display());
    Ok(())
}
