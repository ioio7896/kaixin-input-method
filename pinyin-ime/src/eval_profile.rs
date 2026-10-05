//! Evaluation constructors use the same hot/cold path as the installed service.
use crate::core::PinyinEngine;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EvaluationProfile {
    #[default]
    RuntimeHotCold,
    Full,
}

impl EvaluationProfile {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "runtime-hot-cold" | "runtime" => Ok(Self::RuntimeHotCold),
            "full" => Ok(Self::Full),
            _ => Err(format!(
                "unknown engine profile: {value}; expected runtime-hot-cold or full"
            )),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::RuntimeHotCold => "runtime-hot-cold",
            Self::Full => "full",
        }
    }

    pub fn create_engine(self, dir: &Path) -> PinyinEngine {
        let mut engine = match self {
            Self::RuntimeHotCold => PinyinEngine::with_hot_phrase_dir(Some(dir)),
            Self::Full => PinyinEngine::with_phrase_dir(Some(dir)),
        };
        engine.clear_user_lexicon_for_eval();
        engine
    }

    pub fn report(self, dir: &Path) {
        let cold_available = self == Self::RuntimeHotCold
            && !crate::lexicon_prefs::has_custom_optional_lexicon_prefs()
            && crate::cold_lexicon::ColdLexicon::open(dir).is_some();
        eprintln!(
            "engine_profile={} version={} cold_lexicon={} lexicon_sha256={} lexicon_dir={}",
            self.label(),
            env!("SRF_APP_VERSION"),
            cold_available,
            dataset_sha256(dir).unwrap_or_else(|err| format!("unavailable:{err}")),
            dir.display()
        );
    }
}

fn dataset_sha256(dir: &Path) -> std::io::Result<String> {
    fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                collect(&path, files)?;
            } else if matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("txt" | "bin" | "pack" | "sqlite")
            ) {
                files.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    collect(dir, &mut files)?;
    files.sort();
    let mut hash = Sha256::new();
    for file in files {
        let relative = file
            .strip_prefix(dir)
            .unwrap_or(&file)
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = fs::read(&file)?;
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        hash.update((bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_matches_service_and_unknown_profiles_fail() {
        assert_eq!(
            EvaluationProfile::default(),
            EvaluationProfile::RuntimeHotCold
        );
        assert_eq!(
            EvaluationProfile::parse("runtime-hot-cold"),
            Ok(EvaluationProfile::default())
        );
        assert_eq!(
            EvaluationProfile::parse("full"),
            Ok(EvaluationProfile::Full)
        );
        assert!(EvaluationProfile::parse("ful").is_err());
    }
}
