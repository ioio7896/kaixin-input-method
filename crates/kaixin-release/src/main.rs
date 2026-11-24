use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

mod corpus;
mod lexicon_checks;
mod pack;

type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn read_utf8(path: &Path) -> Result<String> {
    let data = fs::read(path)?;
    let data = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&data);
    Ok(std::str::from_utf8(data)
        .map_err(|error| format!("{}: {error}", path.display()))?
        .to_owned())
}

fn text_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let path = entry.path();
        if kind.is_dir() {
            files.extend(text_files(&path)?);
        } else if kind.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("txt"))
        {
            files.push(path);
        }
    }
    files.sort_unstable();
    Ok(files)
}

fn is_han(ch: char) -> bool {
    matches!(ch as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x2ebef)
}

fn run() -> Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [command, path] if command == "corpus" => corpus::build(Path::new(path)),
        [command, path] if command == "check-lexicons" => lexicon_checks::check(Path::new(path)),
        [command, path] if command == "pack-lexicons" => pack::create(Path::new(path)),
        _ => Err(
            "usage: kaixin-release (corpus|check-lexicons) REPO | pack-lexicons DIRECTORY".into(),
        ),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("kaixin-release: {error}");
        std::process::exit(1);
    }
}
