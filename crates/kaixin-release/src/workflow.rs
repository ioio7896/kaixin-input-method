use chrono::{FixedOffset, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::env;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::options::{Options, Variant};
use crate::{corpus, lexicon_checks, read_utf8, Result};

const HEADLESS: &[&str] = &[
    "srf_ime_engine",
    "srf_ime_setup",
    "srf_ime_clipboard_svc",
    "bake_lexicon",
];
const DESKTOP: &[&str] = &["srf_ime_desktop", "input_perf"];
const NATIVE_DIRS: [&str; 2] = ["tsf-tip/build-package", "tsf-tip/build-package-x86"];

struct Build<'a> {
    options: &'a Options,
    repo: PathBuf,
    target: PathBuf,
    profile: &'static str,
    rust_flags: Option<String>,
}

pub fn run(options: &Options) -> Result<()> {
    let repo = env::current_dir()?;
    if options.dry_run {
        println!("{}", describe(options));
        return Ok(());
    }
    if !cfg!(windows) {
        return Err("the build requires Windows MSVC".into());
    }
    let dist = repo.join("dist");
    fs::create_dir_all(&dist)?;
    let lock = File::options()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dist.join(".release.lock"))?;
    lock.try_lock()
        .map_err(|error| format!("cannot lock the build directory: {error}"))?;

    let build = Build {
        options,
        target: repo.join("pinyin-ime/target"),
        profile: if options.debug { "Debug" } else { "Release" },
        rust_flags: (!options.debug).then(|| release_rustflags(&repo)),
        repo,
    };
    build.execute(&dist)
}

fn describe(options: &Options) -> String {
    if options.check {
        return "Check source encoding, installer sources, corpus and lexicons".into();
    }
    let profile = if options.debug { "Debug" } else { "Release" };
    let rust = if options.skip_cargo {
        "reuse Rust"
    } else {
        "build Rust"
    };
    let native = if options.skip_cmake {
        "reuse x64/x86 TSF"
    } else {
        "build x64/x86 TSF"
    };
    let variants = options
        .variants
        .iter()
        .map(|v| v.name())
        .collect::<Vec<_>>()
        .join(", ");
    let mut steps = format!(
        "{profile}: prepare model resources and lexicons; {rust}; {native}; stage {variants}"
    );
    if options.verify {
        steps.push_str("; run source, Rust and native checks");
    }
    if options.smoke {
        steps.push_str("; validate staged runtime");
    }
    if options.perf {
        steps.push_str("; enforce both input P99 limits");
    }
    if options.release {
        steps.push_str("; publish two Shanghai-timestamped installers and a source ZIP");
    }
    steps
}

impl Build<'_> {
    fn execute(&self, dist: &Path) -> Result<()> {
        let git_prefix = capture(self.command("git").args(["rev-parse", "--show-prefix"]))?;
        if !git_prefix.trim().is_empty() {
            return Err("run cargo xtask from the repository root".into());
        }
        let commit = capture(
            self.command("git")
                .args(["rev-parse", "--short=12", "HEAD"]),
        )?;
        self.check_toolchain()?;
        if self.options.verify {
            self.run(self.cargo().args(["fmt", "--all", "--", "--check"]))?;
            self.platform("Check", &[])?;
        }
        if !self.options.check && (!self.options.skip_cmake || self.options.release) {
            let mut switches = Vec::new();
            if !self.options.skip_cmake {
                switches.push(("-NativeBuild", None));
            }
            if self.options.release {
                switches.push(("-Installers", None));
            }
            self.platform("Preflight", &switches)?;
        }

        let shanghai = Utc::now().with_timezone(&FixedOffset::east_opt(8 * 3600).unwrap());
        let stamp = shanghai.format("%Y%m%d-%H%M%S").to_string();
        let version = if self.options.release {
            let version = format!(
                "{}.{}.{}",
                shanghai.format("%Y"),
                shanghai.format("%m%d").to_string().parse::<u32>()?,
                shanghai.format("%H%M").to_string().parse::<u32>()?
            );
            if self
                .repo
                .join(format!("kaixin-source-{version}.zip"))
                .exists()
                || dist.join(format!("release-{version}.json")).exists()
            {
                return Err(format!(
                    "version {version} was already published; retry in the next minute"
                )
                .into());
            }
            self.platform("Version", &[("-Version", Some(&version))])?;
            println!("Release version {version}");
            version
        } else {
            read_utf8(&self.repo.join("VERSION"))?.trim().to_owned()
        };

        if self.options.clean {
            self.clean()?;
        }
        if self.options.regenerate_lexicons {
            self.platform("RegenerateLexicons", &[])?;
        }
        if self.options.rebuild_corpus {
            remove_file(&self.repo.join("pinyin-ime/data/corpus.txt"))?;
        }
        corpus::build(&self.repo)?;
        if self.options.verify || !self.options.release {
            lexicon_checks::check(&self.repo)?;
        }
        if self.options.check {
            println!("Source and lexicon preparation passed");
            return Ok(());
        }

        self.prepare_model_resources()?;

        let work = dist.join(format!(".release-work-{stamp}-{}", std::process::id()));
        fs::create_dir(&work)?;
        let work_arg = work.to_str().ok_or("build path is not Unicode")?;
        if self.options.verify {
            self.platform("RustTests", &[("-Work", Some(work_arg))])?;
        }
        self.rust_artifacts(commit.trim())?;
        self.native_artifacts(commit.trim())?;
        if self.options.check_skins && !self.options.release {
            self.platform("Skins", &[])?;
        }

        let bin = env::current_exe()?;
        let tool = bin.to_str().ok_or("build tool path is not Unicode")?;
        let mut prepared = None;
        for variant in &self.options.variants {
            let mut args = vec![
                ("-Profile", Some(self.profile)),
                ("-Work", Some(work_arg)),
                ("-Version", Some(version.as_str())),
                ("-Stamp", Some(stamp.as_str())),
                ("-Commit", Some(commit.trim())),
                ("-Variant", Some(variant.name())),
                ("-PackageTool", Some(tool)),
            ];
            if let Some(previous) = prepared {
                args.push(("-PreparedVariant", Some(previous)));
            }
            if self.options.smoke {
                args.push(("-Smoke", None));
            }
            if self.options.portable {
                args.push(("-Portable", None));
            }
            if self.options.release && self.options.smoke && *variant == Variant::Ime {
                args.push(("-RuntimeProfile", None));
            }
            self.platform("Stage", &args)?;
            prepared = Some(variant.name());
        }
        if self.options.perf {
            let p99 = self.options.perf_max_p99.unwrap_or(2000).to_string();
            let incremental_p99 = self
                .options
                .perf_max_incremental_p99
                .unwrap_or(40000)
                .to_string();
            self.platform(
                "Perf",
                &[
                    ("-Work", Some(work_arg)),
                    ("-Variant", prepared),
                    ("-MaxP99", Some(&p99)),
                    ("-MaxIncrementalP99", Some(&incremental_p99)),
                ],
            )?;
        }
        if self.options.collect_logs {
            self.platform("CollectLogs", &[])?;
        }
        if self.options.release {
            let verification = if self.options.verify && self.options.smoke {
                "passed"
            } else {
                "skipped_by_request"
            };
            let mut args = vec![
                ("-Work", Some(work_arg)),
                ("-Version", Some(version.as_str())),
                ("-Stamp", Some(stamp.as_str())),
                ("-Verification", Some(verification)),
            ];
            if self.options.user_installer {
                args.push(("-UserInstaller", None));
            }
            self.platform("Publish", &args)?;
        }
        self.remove_build_directory(&work)?;
        Ok(())
    }

    fn check_toolchain(&self) -> Result<()> {
        let requirements: Value =
            serde_json::from_str(&read_utf8(&self.repo.join("release-toolchain.json"))?)?;
        for name in ["cargo", "rustc"] {
            let installed = capture(self.command(name).arg("--version"))?;
            if installed.split_whitespace().nth(1) != requirements[name].as_str() {
                return Err(format!(
                    "expected {name} {}; found {}",
                    requirements[name],
                    installed.trim()
                )
                .into());
            }
        }
        let host = capture(self.command("rustc").arg("-vV"))?;
        if !host
            .lines()
            .any(|line| line == "host: x86_64-pc-windows-msvc")
        {
            return Err("the build requires x64 Windows MSVC Rust".into());
        }
        Ok(())
    }

    fn prepare_model_resources(&self) -> Result<()> {
        let output = self.target.join("model-data");
        if self.options.skip_cargo {
            let generator = self
                .target
                .join(self.profile.to_ascii_lowercase())
                .join("bake_model.exe");
            return self.run(
                self.command(
                    generator
                        .to_str()
                        .ok_or("model generator path is not Unicode")?,
                )
                .arg(&self.repo)
                .arg(&output),
            );
        }
        let mut command = self.cargo();
        command.args([
            "run",
            "--locked",
            "-p",
            "kaixin-pinyin",
            "--bin",
            "bake_model",
        ]);
        if !self.options.debug {
            command.arg("--release");
        }
        if let Some(flags) = &self.rust_flags {
            command.env("CARGO_ENCODED_RUSTFLAGS", flags);
        }
        self.run(command.arg("--").arg(&self.repo).arg(&output))
    }

    fn rust_artifacts(&self, commit: &str) -> Result<()> {
        if !self.options.skip_cargo {
            self.compile_rust(HEADLESS, true)?;
            self.compile_rust(DESKTOP, false)?;
        }
        let bin = self.target.join(self.profile.to_ascii_lowercase());
        let base = HEADLESS
            .iter()
            .chain(DESKTOP)
            .map(|name| bin.join(format!("{name}.exe")));
        self.artifact_stamp("rust", commit, base, self.options.skip_cargo)
    }

    fn native_artifacts(&self, commit: &str) -> Result<()> {
        if !self.options.skip_cmake {
            let mut args = vec![("-Profile", Some(self.profile))];
            if self.options.verify {
                args.push(("-Verify", None));
            }
            self.platform("Native", &args)?;
        }
        let artifacts = NATIVE_DIRS.into_iter().flat_map(|directory| {
            [
                "srf_tsf_tip.dll",
                "srf_ime_overlay.exe",
                "srf_release_probe.exe",
            ]
            .map(|name| self.repo.join(directory).join(self.profile).join(name))
        });
        self.artifact_stamp("native", commit, artifacts, self.options.skip_cmake)
    }

    fn artifact_stamp(
        &self,
        component: &str,
        commit: &str,
        artifacts: impl IntoIterator<Item = PathBuf>,
        reuse: bool,
    ) -> Result<()> {
        let mut inputs = if component == "native" {
            vec![
                "tsf-tip/CMakeLists.txt",
                "tsf-tip/src",
                "tsf-tip/include",
                "tsf-tip/srf_tsf_tip.def",
                "tsf-tip/tests/release_probe.cpp",
                "shared",
            ]
        } else {
            vec![
                "Cargo.toml",
                "Cargo.lock",
                ".cargo/config.toml",
                "pinyin-ime",
                "crates/kaixin-core",
                "crates/kaixin-pinyin",
                "crates/kaixin-platform",
                "crates/kaixin-clipboard",
                "crates/kaixin-capture",
                "crates/kaixin-translate",
                "crates/kaixin-common",
                "shared",
                "lexicon",
                "assets",
                "icons",
            ]
        };
        inputs.extend(["VERSION", "release-toolchain.json"]);
        let listed = capture(
            self.command("git")
                .args([
                    "ls-files",
                    "-z",
                    "--cached",
                    "--others",
                    "--exclude-standard",
                    "--",
                ])
                .args(inputs),
        )?;
        let mut paths = listed
            .split_terminator('\0')
            .map(|path| self.repo.join(path))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        if component != "native" {
            let external_model_inputs = [
                "pinyin-ime/data/corpus.txt",
                "pinyin-ime/data/pinyin_supported_chars.txt",
                "pinyin-ime/data/syllables.txt",
                "pinyin-ime/data/s2t_chars.sqlite",
                "pinyin-ime/data/symbol_catalog.tsv",
            ];
            paths.retain(|path| {
                !external_model_inputs
                    .iter()
                    .any(|input| path == &self.repo.join(input))
            });
        }
        paths.sort_unstable();
        paths.dedup();
        let mut hash = Sha256::new();
        hash.update(commit.as_bytes());
        if component != "native" {
            hash.update(self.rust_flags.as_deref().unwrap_or("").as_bytes());
            if self.rust_flags.is_none() {
                for name in ["CARGO_ENCODED_RUSTFLAGS", "RUSTFLAGS"] {
                    hash.update(env::var(name).unwrap_or_default());
                    hash.update([0]);
                }
            }
        }
        for path in paths.into_iter().chain(artifacts) {
            hash.update(
                path.strip_prefix(&self.repo)?
                    .as_os_str()
                    .as_encoded_bytes(),
            );
            hash.update([0]);
            let mut file_hash = Sha256::new();
            let mut file =
                File::open(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            io::copy(&mut file, &mut file_hash)?;
            hash.update(file_hash.finalize());
        }
        let fingerprint = format!("{:x}\n", hash.finalize());
        let stamp = self
            .repo
            .join(format!("dist/.build-{component}-{}.sha256", self.profile));
        if reuse {
            if fs::read_to_string(&stamp).ok().as_deref() != Some(&fingerprint) {
                return Err(format!("missing or stale {component} artifacts for {}; build that component before reusing it", self.profile).into());
            }
        } else {
            fs::write(stamp, fingerprint)?;
        }
        Ok(())
    }

    fn compile_rust(&self, binaries: &[&str], headless: bool) -> Result<()> {
        let mut command = self.cargo();
        command.args([
            "build",
            "--locked",
            "-p",
            "pinyin-ime",
            "--features",
            "dev-tools",
        ]);
        if !self.options.debug {
            command.arg("--release");
        }
        if headless {
            command.arg("--no-default-features");
        }
        for binary in binaries {
            command.args(["--bin", binary]);
        }
        if let Some(flags) = &self.rust_flags {
            command.env("CARGO_ENCODED_RUSTFLAGS", flags);
        }
        self.run(&mut command)
    }

    fn clean(&self) -> Result<()> {
        self.remove_build_directory(&self.target)?;
        for directory in NATIVE_DIRS {
            self.remove_build_directory(&self.repo.join(directory))?;
        }
        Ok(())
    }

    fn remove_build_directory(&self, path: &Path) -> Result<()> {
        let resolved = match path.canonicalize() {
            Ok(path) => path,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        let root = self.repo.canonicalize()?;
        if resolved == root || !resolved.starts_with(root) {
            return Err(
                format!("build directory escaped the repository: {}", path.display()).into(),
            );
        }
        fs::remove_dir_all(path)?;
        Ok(())
    }

    fn platform(&self, action: &str, options: &[(&str, Option<&str>)]) -> Result<()> {
        let mut command = self.command("powershell.exe");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(self.repo.join("scripts/release-platform.ps1"))
            .args(["-Action", action]);
        for (name, value) in options {
            command.arg(name).args(value);
        }
        if self.options.sign {
            command.arg("-Sign");
        }
        if self.options.sign_required {
            command.arg("-SignRequired");
        }
        command.env("CARGO_TARGET_DIR", &self.target);
        self.run(&mut command)
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command.current_dir(&self.repo);
        command
    }

    fn cargo(&self) -> Command {
        let mut command = self.command("cargo");
        command.env("CARGO_TARGET_DIR", &self.target);
        command
    }

    fn run(&self, command: &mut Command) -> Result<()> {
        if !self.options.quiet {
            println!("+ {command:?}");
        }
        let status = command.status()?;
        if !status.success() {
            return Err(format!(
                "{} exited with {status}",
                command.get_program().to_string_lossy()
            )
            .into());
        }
        Ok(())
    }
}

fn capture(command: &mut Command) -> Result<String> {
    let Output {
        status,
        stdout,
        stderr,
    } = command.output()?;
    if !status.success() {
        return Err(format!(
            "{} exited with {status}: {}",
            command.get_program().to_string_lossy(),
            String::from_utf8_lossy(&stderr)
        )
        .into());
    }
    Ok(String::from_utf8(stdout)?)
}

fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn release_rustflags(repo: &Path) -> String {
    let user = env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_default();
    let cargo = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| user.join(".cargo"));
    let rustup = env::var_os("RUSTUP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| user.join(".rustup"));
    let plain = env::var("RUSTFLAGS").unwrap_or_default();
    rustflags(
        env::var("CARGO_ENCODED_RUSTFLAGS").ok().as_deref(),
        &plain,
        [
            (user.as_path(), "/build/user"),
            (repo, "/build/kaixin"),
            (cargo.as_path(), "/build/cargo"),
            (rustup.as_path(), "/build/rustup"),
        ],
    )
}

fn rustflags<'a>(
    encoded: Option<&str>,
    plain: &str,
    mappings: impl IntoIterator<Item = (&'a Path, &'a str)>,
) -> String {
    let mut flags = match encoded {
        Some(encoded) => encoded
            .split('\x1f')
            .filter(|flag| !flag.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        None => plain.split_whitespace().map(str::to_owned).collect(),
    };
    for (path, replacement) in mappings {
        let native = path.to_string_lossy();
        if native.is_empty() {
            continue;
        }
        flags.push(format!("--remap-path-prefix={native}={replacement}"));
        let slash = native.replace('\\', "/");
        if slash != native {
            flags.push(format!("--remap-path-prefix={slash}={replacement}"));
        }
    }
    flags.join("\x1f")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_rustflags_preserve_precedence_and_paths_with_spaces() {
        let flags = rustflags(
            Some("--cfg\x1fexample=\"two words\""),
            "--cfg ignored",
            [(Path::new("G:\\workspace with spaces"), "/build/kaixin")],
        );
        assert_eq!(
            flags.split('\x1f').collect::<Vec<_>>(),
            [
                "--cfg",
                "example=\"two words\"",
                "--remap-path-prefix=G:\\workspace with spaces=/build/kaixin",
                "--remap-path-prefix=G:/workspace with spaces=/build/kaixin",
            ]
        );
        assert_eq!(rustflags(Some(""), "--cfg ignored", []), "");
        assert_eq!(rustflags(None, "-C debuginfo=1", []), "-C\x1fdebuginfo=1");
    }
}
