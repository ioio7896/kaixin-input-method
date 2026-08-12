use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

pub fn read_text_file(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    decode_text_bytes(&bytes).map_err(|message| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{message}: {}", path.display()),
        )
    })
}

pub fn decode_text_bytes(bytes: &[u8]) -> Result<String, String> {
    let (encoding, payload) = detect_text_encoding(bytes).ok_or_else(|| {
        "text is neither valid UTF-8 nor a recognizable UTF-16 variant".to_string()
    })?;
    match encoding {
        TextEncoding::Utf8 => {
            String::from_utf8(payload.to_vec()).map_err(|err| format!("invalid utf-8: {err}"))
        }
        TextEncoding::Utf16Le => decode_utf16_bytes(payload, true),
        TextEncoding::Utf16Be => decode_utf16_bytes(payload, false),
    }
}

fn detect_text_encoding(bytes: &[u8]) -> Option<(TextEncoding, &[u8])> {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return Some((TextEncoding::Utf8, rest));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Some((TextEncoding::Utf16Le, rest));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Some((TextEncoding::Utf16Be, rest));
    }
    if looks_like_utf16(bytes, true) && decode_utf16_bytes(bytes, true).is_ok() {
        return Some((TextEncoding::Utf16Le, bytes));
    }
    if looks_like_utf16(bytes, false) && decode_utf16_bytes(bytes, false).is_ok() {
        return Some((TextEncoding::Utf16Be, bytes));
    }
    if std::str::from_utf8(bytes).is_ok() {
        return Some((TextEncoding::Utf8, bytes));
    }
    None
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
