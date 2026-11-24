use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use crate::Result;

const MAGIC: &[u8; 8] = b"KXLP0001";
const PROFILE_MAGIC: &[u8; 8] = b"SRFLX002";
const MAX_PROFILE_BYTES: usize = 512 * 1024 * 1024;

fn compress(data: &[u8]) -> Result<Vec<u8>> {
    if !data.starts_with(PROFILE_MAGIC) || data.len() > MAX_PROFILE_BYTES || data.len() < 12 {
        return Err("invalid prebaked lexicon profile".into());
    }
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

fn compare_profile<R: Read>(input: R, compressed_len: usize, original: &[u8]) -> Result<()> {
    let mut decoder = ZlibDecoder::new(input.take(compressed_len as u64));
    let mut buffer = [0; 65536];
    for expected in original.chunks(buffer.len()) {
        let slice = &mut buffer[..expected.len()];
        decoder.read_exact(slice)?;
        if slice != expected {
            return Err("lexicon pack round-trip differs from source".into());
        }
    }
    if decoder.read(&mut buffer[..1])? != 0 || decoder.total_in() != compressed_len as u64 {
        return Err("lexicon pack profile length mismatch".into());
    }
    Ok(())
}

pub fn create(root: &Path) -> Result<()> {
    let paths = [root.join("lexicon.bin"), root.join("hot_lexicon.bin")];
    let standard = fs::read(&paths[0])?;
    let hot = fs::read(&paths[1])?;
    if standard.get(8..12) != hot.get(8..12) {
        return Err("standard and hot lexicon schemas differ".into());
    }
    let compressed_standard = compress(&standard)?;
    let compressed_hot = compress(&hot)?;
    let pending = root.join("lexicon.pack.tmp");
    let write = || -> Result<()> {
        let mut output = BufWriter::new(File::create(&pending)?);
        output.write_all(MAGIC)?;
        for len in [
            compressed_standard.len(),
            compressed_hot.len(),
            standard.len(),
            hot.len(),
        ] {
            output.write_all(&(len as u64).to_le_bytes())?;
        }
        output.write_all(&compressed_standard)?;
        output.write_all(&compressed_hot)?;
        output.flush()?;
        drop(output);

        let mut input = File::open(&pending)?;
        let mut header = [0; 40];
        input.read_exact(&mut header)?;
        if &header[..8] != MAGIC {
            return Err("invalid lexicon pack header".into());
        }
        compare_profile(&mut input, compressed_standard.len(), &standard)?;
        compare_profile(&mut input, compressed_hot.len(), &hot)?;
        drop(input);
        fs::rename(&pending, root.join("lexicon.pack"))?;
        Ok(())
    };
    if let Err(error) = write() {
        let _ = fs::remove_file(&pending);
        return Err(error);
    }
    for path in paths {
        fs::remove_file(path)?;
    }
    println!(
        "Lossless lexicon pack: {} -> {} bytes",
        standard.len() + hot.len(),
        40 + compressed_standard.len() + compressed_hot.len()
    );
    Ok(())
}
