use crate::Result;

pub const HELP: &str = "\
cargo xtask release [--sign|--sign-required] [--user-installer] [--skip-tests]
cargo xtask build [--debug] [--no-inno] [--package-variants all|ime|ocr]
                  [--quick|--skip-cargo|--skip-cmake] [--clean]
                  [--portable-zip] [--perf-smoke] [--verify|--no-verify]
                  [--smoke|--no-smoke] [--skip-runtime-checks] [--dry-run]
cargo xtask check
The default build publishes two timestamped installers and a source ZIP.
Debug, --no-inno, --quick, component skips, and single variants stage only.
Quick staging reuses artifacts verified by a previous build at the same version.
";

#[derive(Copy, Clone, PartialEq, Eq)]
pub enum Variant {
    Ime,
    Ocr,
}

impl Variant {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ime => "ime",
            Self::Ocr => "ocr",
        }
    }
}

#[derive(Default)]
pub struct Options {
    pub release: bool,
    pub check: bool,
    pub help: bool,
    pub debug: bool,
    pub quick: bool,
    pub skip_cargo: bool,
    pub skip_cmake: bool,
    pub clean: bool,
    pub rebuild_corpus: bool,
    pub sign: bool,
    pub sign_required: bool,
    pub user_installer: bool,
    pub portable: bool,
    pub dry_run: bool,
    pub quiet: bool,
    pub verify: bool,
    pub smoke: bool,
    pub perf: bool,
    pub perf_max_p99: Option<u32>,
    pub perf_max_incremental_p99: Option<u32>,
    pub regenerate_lexicons: bool,
    pub collect_logs: bool,
    pub check_skins: bool,
    pub variants: Vec<Variant>,
}

pub fn parse(command: &str, args: &[String]) -> Result<Options> {
    let mut options = Options {
        check: command == "check",
        variants: vec![Variant::Ime, Variant::Ocr],
        check_skins: true,
        ..Options::default()
    };
    let (mut stage_only, mut skip_tests, mut skip_runtime) = (false, false, false);
    let (mut verify, mut smoke) = (None, None);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let (name, value) = arg.split_once('=').unwrap_or((arg, ""));
        let mut flag_value = || {
            if !value.is_empty() {
                return Ok(value);
            }
            args.next()
                .map(String::as_str)
                .ok_or_else(|| format!("{name} requires a value"))
        };
        match name {
            "--help" | "-h" | "-Help" => options.help = true,
            "--check" | "-Check" => options.check = true,
            "--debug" => options.debug = true,
            "--quick" => options.quick = true,
            "--skip-cargo" => options.skip_cargo = true,
            "--skip-cmake" => options.skip_cmake = true,
            "--no-inno" => stage_only = true,
            "--clean" => options.clean = true,
            "--rebuild-corpus" => options.rebuild_corpus = true,
            "--regenerate-lexicons" => options.regenerate_lexicons = true,
            "--no-skin-check" => options.check_skins = false,
            "--sign" | "-Sign" => options.sign = true,
            "--sign-required" | "-SignRequired" => options.sign_required = true,
            "--user-installer" | "-UserInstaller" => options.user_installer = true,
            "--portable-zip" | "--zip" => options.portable = true,
            "--dry-run" => options.dry_run = true,
            "--quiet" | "-q" => options.quiet = true,
            "--verbose" | "-v" => options.quiet = false,
            "--verify" => verify = Some(true),
            "--no-verify" => verify = Some(false),
            "--smoke" => smoke = Some(true),
            "--no-smoke" => smoke = Some(false),
            "--skip-runtime-checks" => skip_runtime = true,
            "--skip-tests" | "-SkipTests" => skip_tests = true,
            "--perf-smoke" => options.perf = true,
            "--perf-max-p99-us" => options.perf_max_p99 = Some(positive_number(flag_value()?)?),
            "--perf-max-incremental-p99-us" => {
                options.perf_max_incremental_p99 = Some(positive_number(flag_value()?)?);
            }
            "--collect-runtime-logs" => options.collect_logs = true,
            "--package-variants" => options.variants = variants(flag_value()?)?,
            // These switches are retained by the Python and cmd entry points.
            "--force" | "--no-parallel" | "--include-translation" | "--no-translation" => {}
            "--no-version" => stage_only = true,
            _ => return Err(format!("unknown option: {arg}").into()),
        }
    }
    options.skip_cargo |= options.quick;
    options.skip_cmake |= options.quick;
    stage_only |=
        options.debug || options.skip_cargo || options.skip_cmake || options.variants.len() != 2;
    options.release = !options.check && !stage_only;
    options.verify = options.check || (!skip_tests && verify.unwrap_or(options.release));
    options.smoke = !skip_tests && !skip_runtime && smoke.unwrap_or(options.release);

    if !options.help {
        if command == "release" && stage_only {
            return Err(
                "release requires a full build of both variants; use cargo xtask build for staging"
                    .into(),
            );
        }
        if options.clean && (options.skip_cargo || options.skip_cmake) {
            return Err(
                "--clean rebuilds all components and cannot be used with component skips".into(),
            );
        }
        if options.verify && !options.check && (options.skip_cargo || options.skip_cmake) {
            return Err("--verify requires compiling both Rust and native targets".into());
        }
        if options.perf && options.debug {
            return Err("--perf-smoke requires Release artifacts".into());
        }
    }
    Ok(options)
}

fn positive_number(value: &str) -> Result<u32> {
    let number = value.parse()?;
    if number == 0 {
        return Err("performance limits must be greater than zero".into());
    }
    Ok(number)
}

fn variants(value: &str) -> Result<Vec<Variant>> {
    let (mut ime, mut ocr) = (false, false);
    for part in value.split(',') {
        match part.trim().to_ascii_lowercase().as_str() {
            "all" => (ime, ocr) = (true, true),
            "ime" => ime = true,
            "ocr" | "full" => ocr = true,
            _ => return Err(format!("unknown package variant: {part}").into()),
        }
    }
    Ok([ime.then_some(Variant::Ime), ocr.then_some(Variant::Ocr)]
        .into_iter()
        .flatten()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(command: &str, flags: &[&str]) -> Result<Options> {
        parse(
            command,
            &flags.iter().map(|flag| (*flag).into()).collect::<Vec<_>>(),
        )
    }

    #[test]
    fn staging_reuses_a_version_without_running_release_verification() {
        let stage = options("build", &["--quick", "--package-variants=ime"]).unwrap();
        assert!(stage.skip_cargo && stage.skip_cmake);
        assert!(!stage.release && !stage.verify && !stage.smoke);
        assert_eq!(stage.variants.len(), 1);
        assert!(options("release", &["--quick"]).is_err());
    }

    #[test]
    fn supported_entry_points_select_the_same_full_release() {
        for command in ["build", "release"] {
            let release = options(command, &[]).unwrap();
            assert!(release.release && release.verify && release.smoke);
            assert_eq!(release.variants.len(), 2);
        }
        let release = options("release", &["-SkipTests"]).unwrap();
        assert!(release.release && !release.verify && !release.smoke);
    }
}
