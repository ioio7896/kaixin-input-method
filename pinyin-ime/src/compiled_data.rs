use crate::lm::Lm;
use crate::runtime_log::{self, RuntimeLogLevel};
use std::collections::{HashMap, HashSet};
use std::io::{Cursor, Read};
use std::sync::{Arc, OnceLock};

const MODEL_MAGIC: &[u8; 8] = b"SRFMD001";
const MODEL_SCHEMA_VERSION: u32 = 1;

/// Process-wide immutable model shared by every lookup session.
pub(crate) struct EngineModel {
    pub dict: Arc<HashMap<String, Vec<char>>>,
    pub syllables: Arc<HashSet<String>>,
    pub lm: Arc<Lm>,
    pub char_count: usize,
}

pub(crate) fn shared_engine_model() -> &'static EngineModel {
    static MODEL: OnceLock<EngineModel> = OnceLock::new();
    MODEL.get_or_init(load_compiled_model)
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

fn read_string<R: Read>(reader: &mut R) -> Result<String, String> {
    let len = read_count(reader)?;
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

fn load_compiled_model() -> EngineModel {
    match try_load_compiled_model() {
        Ok(model) => model,
        Err(err) => {
            runtime_log::log_engine(
                RuntimeLogLevel::Error,
                "compiled_model_load_failed",
                format!("fallback=empty reason={err}"),
            );
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
    let bytes = include_bytes!(concat!(env!("OUT_DIR"), "/compiled_model.bin"));
    let mut reader = Cursor::new(bytes.as_slice());

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

    let dict_len = read_count(&mut reader)?;
    let mut dict = HashMap::with_capacity(dict_len);
    for _ in 0..dict_len {
        let key = read_string(&mut reader)?;
        let item_len = read_count(&mut reader)?;
        let mut chars = Vec::with_capacity(item_len);
        for _ in 0..item_len {
            chars.push(read_char(&mut reader)?);
        }
        dict.insert(key, chars);
    }

    let syllable_len = read_count(&mut reader)?;
    let mut syllables = HashSet::with_capacity(syllable_len);
    for _ in 0..syllable_len {
        syllables.insert(read_string(&mut reader)?);
    }

    let unigram_len = read_count(&mut reader)?;
    let mut unigram = HashMap::with_capacity(unigram_len);
    for _ in 0..unigram_len {
        let ch = read_char(&mut reader)?;
        let count = read_count(&mut reader)?;
        unigram.insert(ch, count);
    }

    let bigram_len = read_count(&mut reader)?;
    let mut bigram = HashMap::with_capacity(bigram_len);
    for _ in 0..bigram_len {
        let left = read_char(&mut reader)?;
        let right = read_char(&mut reader)?;
        let count = read_count(&mut reader)?;
        bigram.insert((left, right), count);
    }

    let vocab: HashSet<char> = dict.values().flatten().copied().collect();
    let lm = Lm::from_counts(vocab, unigram, bigram);

    Ok(EngineModel {
        dict: Arc::new(dict),
        syllables: Arc::new(syllables),
        lm: Arc::new(lm),
        char_count,
    })
}
