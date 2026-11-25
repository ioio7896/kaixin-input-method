use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

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

fn main() {
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let repo = root.join("../..");
    let version_path = repo.join("VERSION");
    let version = fs::read_to_string(&version_path).expect("read VERSION");
    let version = version.trim();
    assert_eq!(version, env!("CARGO_PKG_VERSION"));
    println!("cargo:rerun-if-changed={}", version_path.display());
    for path in git_state_paths(&repo) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!(
        "cargo:rustc-env=KAIXIN_SOURCE_DIR={}",
        repo.join("pinyin-ime").display()
    );
    println!("cargo:rustc-env=SRF_APP_VERSION={version}");
    println!(
        "cargo:rustc-env=SRF_ENGINE_GIT_COMMIT={}",
        git_commit_short(&repo)
    );
    println!(
        "cargo:rustc-env=SRF_ENGINE_GIT_DIRTY={}",
        u8::from(git_dirty(&repo))
    );
}
