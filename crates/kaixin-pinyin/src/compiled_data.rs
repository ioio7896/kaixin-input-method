use crate::lm::Lm;
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::sync::{Arc, OnceLock};

use crate::model_resources::{
    open_resource, BIGRAM_ENTRY_BYTES, MODEL_FILE, MODEL_MAGIC, MODEL_SCHEMA_VERSION,
};

/// Process-wide immutable model shared by every lookup session.
pub struct EngineModel {
    pub dict: Arc<HashMap<String, Vec<char>>>,
    pub syllables: Arc<HashSet<String>>,
    pub lm: Arc<Lm>,
    pub char_count: usize,
}

static MODEL_LOAD_ERROR: OnceLock<String> = OnceLock::new();

pub fn shared_engine_model() -> &'static EngineModel {
    static MODEL: OnceLock<EngineModel> = OnceLock::new();
    MODEL.get_or_init(load_compiled_model)
}

pub fn compiled_model_error() -> Option<&'static str> {
    let _ = shared_engine_model();
    MODEL_LOAD_ERROR.get().map(String::as_str)
}

fn read_u32<R: Read>(reader: &mut R) -> Result<u32, String> {
    let mut bytes = [0u8; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|err| format!("read u32: {err}"))?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_count<R: Read>(reader: &mut R) -> Result<usize, String> {
    usize::try_from(read_u32(reader)?).map_err(|_| "count does not fit usize".to_string())
}

fn read_string(reader: &mut Cursor<&[u8]>) -> Result<String, String> {
    let len = read_count(reader)?;
    if len > 64 {
        return Err("compiled reading exceeds 64 bytes".into());
    }
    let mut bytes = vec![0u8; len];
    reader
        .read_exact(&mut bytes)
        .map_err(|err| format!("read string bytes: {err}"))?;
    String::from_utf8(bytes).map_err(|err| format!("compiled model string is not utf-8: {err}"))
}

fn read_char<R: Read>(reader: &mut R) -> Result<char, String> {
    let value = read_u32(reader)?;
    char::from_u32(value).ok_or_else(|| format!("compiled model char is invalid: {value:#x}"))
}

fn read_bounded_count(
    reader: &mut Cursor<&[u8]>,
    bytes_per_record: usize,
) -> Result<usize, String> {
    let count = read_count(reader)?;
    let remaining = reader
        .get_ref()
        .len()
        .saturating_sub(reader.position() as usize);
    if count > remaining / bytes_per_record {
        return Err("compiled model table exceeds file length".into());
    }
    Ok(count)
}

fn load_compiled_model() -> EngineModel {
    match try_load_compiled_model() {
        Ok(model) => model,
        Err(err) => {
            let _ = MODEL_LOAD_ERROR.set(err.clone());
            eprintln!("compiled pinyin model unavailable: {err}");
            empty_compiled_model()
        }
    }
}

fn empty_compiled_model() -> EngineModel {
    EngineModel {
        dict: Arc::new(HashMap::new()),
        syllables: Arc::new(HashSet::new()),
        lm: Arc::new(Lm::from_counts(
            HashSet::new(),
            HashMap::new(),
            HashMap::new(),
        )),
        char_count: 0,
    }
}

fn try_load_compiled_model() -> Result<EngineModel, String> {
    let mapped = Arc::new(open_resource(MODEL_FILE).map_err(|err| err.to_string())?);
    let mut reader = Cursor::new(&mapped[..]);

    let mut magic = [0u8; 8];
    reader
        .read_exact(&mut magic)
        .map_err(|err| format!("read model magic: {err}"))?;
    if &magic != MODEL_MAGIC {
        return Err("unexpected compiled model magic".to_string());
    }
    let schema_version = read_u32(&mut reader)?;
    if schema_version != MODEL_SCHEMA_VERSION {
        return Err(format!(
            "unsupported compiled model schema version: {schema_version}"
        ));
    }

    let char_count = read_count(&mut reader)?;

    let dict_len = read_bounded_count(&mut reader, 8)?;
    let mut dict = HashMap::with_capacity(dict_len);
    for _ in 0..dict_len {
        let key = read_string(&mut reader)?;
        let item_len = read_bounded_count(&mut reader, 4)?;
        let mut chars = Vec::with_capacity(item_len);
        for _ in 0..item_len {
            chars.push(read_char(&mut reader)?);
        }
        dict.insert(key, chars);
    }

    let syllable_len = read_bounded_count(&mut reader, 4)?;
    let mut syllables = HashSet::with_capacity(syllable_len);
    for _ in 0..syllable_len {
        syllables.insert(read_string(&mut reader)?);
    }

    let unigram_len = read_bounded_count(&mut reader, 8)?;
    let mut unigram = HashMap::with_capacity(unigram_len);
    for _ in 0..unigram_len {
        let ch = read_char(&mut reader)?;
        let count = read_count(&mut reader)?;
        unigram.insert(ch, count);
    }

    let bigram_len = read_bounded_count(&mut reader, BIGRAM_ENTRY_BYTES)?;
    let start = reader.position() as usize;
    let end = start + bigram_len * BIGRAM_ENTRY_BYTES;
    if end != mapped.len() {
        return Err("unexpected bytes after the compiled model".into());
    }
    let vocab: HashSet<char> = dict.values().flatten().copied().collect();
    let lm = Lm::from_mapped_bigrams(vocab, unigram, mapped, start..end);

    Ok(EngineModel {
        dict: Arc::new(dict),
        syllables: Arc::new(syllables),
        lm: Arc::new(lm),
        char_count,
    })
}
