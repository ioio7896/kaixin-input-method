use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "src/dict.rs"]
mod dict;

const MODEL_MAGIC: &[u8; 8] = b"SRFMD001";
const MODEL_SCHEMA_VERSION: u32 = 1;

fn fnv1a64_update(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1099511628211);
    }
    hash
}

fn fnv1a64_hex(parts: &[&[u8]]) -> String {
    let mut hash = 1469598103934665603u64;
    for part in parts {
        hash = fnv1a64_update(hash, part);
    }
    format!("{hash:016x}")
}

fn git_output(repo: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn git_commit_short(repo: &Path) -> String {
    git_output(repo, &["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".into())
}

fn git_dirty(repo: &Path) -> bool {
    git_output(repo, &["status", "--porcelain"]).is_some_and(|text| !text.trim().is_empty())
}

/// Git files whose change should re-run the build script so the embedded
/// commit stamp tracks HEAD even when no source file changed.  Uses the real
/// git dir (worktree-safe); `packed-refs` covers ref-pack rewrites.  Not
/// watching `.git/index`: `git add` without a commit would re-stamp the
/// engine while the TIP DLL (stamped at CMake configure time) stays put.
fn git_state_paths(repo: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(text) = git_output(repo, &["rev-parse", "--absolute-git-dir"]) {
        let dir = PathBuf::from(text);
        for name in ["HEAD", "packed-refs"] {
            let path = dir.join(name);
            if path.is_file() {
                paths.push(path);
            }
        }
    }
    paths
}

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

    for pair in bytes.chunks_exact(2) {
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

    let units = bytes.chunks_exact(2).map(|pair| {
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
) -> Result<(), String> {
    let dict = dict::build_dict(chars);
    let vocab: HashSet<char> = dict.values().flatten().copied().collect();
    let syllable_list = load_syllables(syllables);
    let (unigram, bigram) = build_lm_counts(corpus, &vocab);
    let char_count = chars.chars().filter(|ch| !ch.is_whitespace()).count();

    let path = out_dir.join("compiled_model.bin");
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
    for (ch, count) in unigram_entries {
        write_char(&mut writer, ch);
        write_count(&mut writer, count);
    }

    let mut bigram_entries: Vec<_> = bigram.into_iter().collect();
    bigram_entries.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    write_count(&mut writer, bigram_entries.len());
    for ((left, right), count) in bigram_entries {
        write_char(&mut writer, left);
        write_char(&mut writer, right);
        write_count(&mut writer, count);
    }

    writer
        .flush()
        .map_err(|e| format!("flush compiled model: {e}"))?;
    Ok(())
}

/// 发布后嵌入 `compiled_model.bin` 的结构门禁：与 `compiled_data.rs` 解析约定一致。
const VALIDATE_MAX_STRING_BYTES: usize = 1 << 26;

fn read_u32_validate(cursor: &mut Cursor<&[u8]>) -> Result<u32, String> {
    let mut b = [0u8; 4];
    cursor
        .read_exact(&mut b)
        .map_err(|e| format!("read u32: {e}"))?;
    Ok(u32::from_le_bytes(b))
}

fn skip_string_validate(cursor: &mut Cursor<&[u8]>) -> Result<(), String> {
    let len = read_u32_validate(cursor)? as usize;
    if len > VALIDATE_MAX_STRING_BYTES {
        return Err(format!(
            "compiled model string length {len} exceeds max {VALIDATE_MAX_STRING_BYTES}"
        ));
    }
    let pos = cursor.stream_position().map_err(|e| format!("tell: {e}"))?;
    let end = pos
        .checked_add(len as u64)
        .ok_or_else(|| "compiled model string length overflow".to_string())?;
    let file_len = cursor.get_ref().len() as u64;
    if end > file_len {
        return Err("compiled model truncated inside string".to_string());
    }
    cursor
        .seek(SeekFrom::Current(len as i64))
        .map_err(|e| format!("skip string: {e}"))?;
    Ok(())
}

fn validate_compiled_model_bytes(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 8 + 4 {
        return Err("compiled model too small".to_string());
    }
    let mut cursor = Cursor::new(bytes);
    let mut magic = [0u8; 8];
    cursor
        .read_exact(&mut magic)
        .map_err(|e| format!("read magic: {e}"))?;
    if &magic != MODEL_MAGIC {
        return Err("compiled model magic mismatch".to_string());
    }
    let ver = read_u32_validate(&mut cursor)?;
    if ver != MODEL_SCHEMA_VERSION {
        return Err(format!(
            "compiled model schema {ver}, expected {MODEL_SCHEMA_VERSION}"
        ));
    }

    let char_count = read_u32_validate(&mut cursor)? as usize;
    let _ = char_count;

    let dict_len = read_u32_validate(&mut cursor)? as usize;
    for _ in 0..dict_len {
        skip_string_validate(&mut cursor)?;
        let item_len = read_u32_validate(&mut cursor)? as usize;
        for _ in 0..item_len {
            let _ch = read_u32_validate(&mut cursor)?;
        }
    }

    let syllable_len = read_u32_validate(&mut cursor)? as usize;
    for _ in 0..syllable_len {
        skip_string_validate(&mut cursor)?;
    }

    let unigram_len = read_u32_validate(&mut cursor)? as usize;
    for _ in 0..unigram_len {
        let _ch = read_u32_validate(&mut cursor)?;
        let _cnt = read_u32_validate(&mut cursor)? as usize;
    }

    let bigram_len = read_u32_validate(&mut cursor)? as usize;
    for _ in 0..bigram_len {
        let _l = read_u32_validate(&mut cursor)?;
        let _r = read_u32_validate(&mut cursor)?;
        let _cnt = read_u32_validate(&mut cursor)? as usize;
    }

    let pos = cursor
        .stream_position()
        .map_err(|e| format!("tell end: {e}"))?;
    if pos != bytes.len() as u64 {
        return Err(format!(
            "compiled model has trailing bytes (at {pos}, len {})",
            bytes.len()
        ));
    }
    Ok(())
}

fn version_resource_numbers(version: &str) -> [u16; 4] {
    let mut out = [0u16; 4];
    for (idx, part) in version.split('.').take(4).enumerate() {
        let digits: String = part.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        out[idx] = digits.parse::<u16>().unwrap_or(0);
    }
    out
}

fn rc_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn compile_windows_app_resource(out_dir: &Path, manifest_dir: &Path, app_version: &str) {
    #[cfg(windows)]
    {
        let repo_root = manifest_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| manifest_dir.to_path_buf());
        let icon_path = repo_root.join("assets").join("kaixin-input.ico");
        if !icon_path.is_file() {
            println!(
                "cargo:warning=Windows icon not found, skip embedding: {}",
                icon_path.display()
            );
            return;
        }

        let icon_path: PathBuf = icon_path
            .canonicalize()
            .unwrap_or_else(|_| icon_path.clone());
        let icon_escaped = rc_string(&icon_path.display().to_string());
        let version_numbers = version_resource_numbers(app_version);
        let app_name = "\u{5f00}\u{5fc3}\u{8f93}\u{5165}\u{6cd5}";
        let company_name = "\u{5f00}\u{5fc3}\u{8f93}\u{5165}\u{6cd5}";
        let rc_path = out_dir.join("kaixin_app.rc");
        let rc_content = format!(
            r#"#pragma code_page(65001)
1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {v0},{v1},{v2},{v3}
PRODUCTVERSION {v0},{v1},{v2},{v3}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "080404B0"
        BEGIN
            VALUE "CompanyName", "{company}"
            VALUE "FileDescription", "{app}"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "{app}"
            VALUE "OriginalFilename", "{app}"
            VALUE "ProductName", "{app}"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x0804, 1200
    END
END
"#,
            icon = icon_escaped,
            v0 = version_numbers[0],
            v1 = version_numbers[1],
            v2 = version_numbers[2],
            v3 = version_numbers[3],
            company = rc_string(company_name),
            app = rc_string(app_name),
            version = rc_string(app_version),
        );
        if let Err(err) = fs::write(&rc_path, rc_content) {
            println!(
                "cargo:warning=Failed to write Windows resource script: {} ({err})",
                rc_path.display()
            );
            return;
        }

        let result =
            embed_resource::compile(rc_path.to_string_lossy().as_ref(), embed_resource::NONE);
        if let Err(err) = result.manifest_optional() {
            println!("cargo:warning=Failed to compile Windows app resource: {err}");
        } else {
            println!("cargo:rerun-if-changed={}", icon_path.display());
            println!("cargo:rerun-if-changed={}", rc_path.display());
        }
    }
}

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("manifest dir");
    let out_dir = env::var("OUT_DIR").expect("out dir");
    let manifest = Path::new(&manifest);
    let out_dir = Path::new(&out_dir);
    let repo = manifest.parent().unwrap_or(manifest);

    let supported_chars = manifest.join("data/pinyin_supported_chars.txt");
    let corpus = manifest.join("data/corpus.txt");
    let s2t_database = manifest.join("data/s2t_chars.sqlite");
    let syllables = manifest.join("data/syllables.txt");
    let version_path = repo.join("VERSION");
    let app_version = fs::read_to_string(&version_path)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "dev".into()));
    let cargo_pkg_version = env::var("CARGO_PKG_VERSION").unwrap_or_default();
    if !cargo_pkg_version.is_empty() && app_version != cargo_pkg_version {
        panic!(
            "VERSION ({app_version}) must match pinyin-ime/Cargo.toml package.version ({cargo_pkg_version})"
        );
    }

    let chars_raw = fs::read(&supported_chars).expect("read pinyin_supported_chars.txt");
    let chars_text = decode_text_bytes(&chars_raw).expect("decode pinyin_supported_chars.txt");
    let chars_path = out_dir.join("chars_pinyin_supported.txt");
    fs::write(&chars_path, &chars_text).expect("write chars_pinyin_supported");

    let staged_s2t_database = out_dir.join("s2t_chars.sqlite");
    match fs::read(&s2t_database) {
        Ok(bytes) => fs::write(&staged_s2t_database, bytes).expect("stage s2t_chars.sqlite"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "cargo:warning={} is not available; Traditional conversion will use identity mapping",
                s2t_database.display()
            );
            fs::write(&staged_s2t_database, []).expect("write empty s2t placeholder");
        }
        Err(err) => panic!("read s2t database {}: {err}", s2t_database.display()),
    }
    println!("cargo:rerun-if-changed={}", s2t_database.display());

    let corpus_text = match fs::read(&corpus) {
        Ok(bytes) => decode_text_bytes(&bytes).expect("decode corpus"),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "cargo:warning={} is not generated yet; compiling with an empty corpus",
                corpus.display()
            );
            String::new()
        }
        Err(err) => panic!("read corpus {}: {err}", corpus.display()),
    };
    let syllables_text = decode_text_bytes(&fs::read(&syllables).expect("read syllables"))
        .expect("decode syllables");
    write_compiled_model(out_dir, &chars_text, &corpus_text, &syllables_text)
        .expect("write compiled model");

    let compiled_path = out_dir.join("compiled_model.bin");
    let compiled_bytes = fs::read(&compiled_path).expect("read compiled model for validation");
    validate_compiled_model_bytes(&compiled_bytes).expect("compiled_model.bin validation failed");
    let model_hash = fnv1a64_hex(&[
        chars_text.as_bytes(),
        corpus_text.as_bytes(),
        syllables_text.as_bytes(),
        &compiled_bytes,
    ]);
    println!("cargo:rustc-env=SRF_APP_VERSION={app_version}");
    println!(
        "cargo:rustc-env=SRF_ENGINE_GIT_COMMIT={}",
        git_commit_short(repo)
    );
    println!(
        "cargo:rustc-env=SRF_ENGINE_GIT_DIRTY={}",
        if git_dirty(repo) { "1" } else { "0" }
    );
    for path in git_state_paths(repo) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rustc-env=SRF_ENGINE_MODEL_HASH={model_hash}");
    compile_windows_app_resource(out_dir, manifest, &app_version);

    println!("cargo:rerun-if-changed={}", supported_chars.display());
    println!("cargo:rerun-if-changed={}", corpus.display());
    println!("cargo:rerun-if-changed={}", syllables.display());
    println!("cargo:rerun-if-changed={}", version_path.display());
    println!("cargo:rerun-if-changed={}", manifest.join("src").display());
}
