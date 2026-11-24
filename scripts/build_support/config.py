from __future__ import annotations
import json
import re
from pathlib import Path
from dataclasses import dataclass
from collections import OrderedDict

VERSION = (Path(__file__).resolve().parents[2] / "VERSION").read_text(encoding="utf-8").strip()
RETIRED_OPTIONAL_LEXICON_TAGS = frozenset(
    json.loads(
        (
            Path(__file__).resolve().parents[2] / "shared" / "retired_optional_lexicons.json"
        ).read_text(encoding="utf-8")
    )
)
RETIRED_LEXICON_FILES = tuple(
    json.loads(
        (Path(__file__).resolve().parents[2] / "shared" / "retired_lexicon_files.json").read_text(
            encoding="utf-8"
        )
    )
)
VERSION_RE = re.compile(r"^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$")

# ---------------------------------------------------------------------------


def _read_version() -> str:
    """Build helper: _read_version."""
    version_file = Path(__file__).resolve().parents[2] / "VERSION"
    if version_file.is_file():
        try:
            return version_file.read_text(encoding="utf-8").strip()
        except Exception:
            pass
    return VERSION


ARCH_X64 = "x64"
ARCH_X86 = "x86"
BUILD_ARCHES = (ARCH_X64, ARCH_X86)
PACKAGE_HASH_MANIFEST = "package_manifest.sha256"
COMPONENT_MANIFEST = "component_manifest.ini"
COMPONENT_PREFIXES = OrderedDict(
    (
        ("python_runtime", ".python-runtime/"),
        ("rapidocr_packages", ".python-packages/"),
        ("rapidocr_payload", "RapidOCRLite/"),
    )
)
PREBAKED_LEXICON_MAGIC = b"SRFLX002"
# Schema 13 preserves each indexed reading's weight and pronunciation class.
PREBAKED_LEXICON_SCHEMA_VERSION = 14
PREBAKED_LEXICON_HEADER_SIZE = len(PREBAKED_LEXICON_MAGIC) + 4
PREBAKED_LEXICON_MIN_BYTES = 1000
COMMON_ENGLISH_LEXICON_RELATIVE_PATH = Path("en") / "kaixin_common_english.txt"
COMMON_ENGLISH_LICENSE_RELATIVE_PATH = Path("en") / "LICENSE.wordfreq.md"
EXPECTED_COMMON_ENGLISH_WORDS = 20_000
SINGLE_CHAR_COMMON_RELATIVE_PATH = Path("zh") / "single_char_common_8105.txt"
EXPECTED_SINGLE_CHAR_COMMON_ROWS = 8_105
REQUIRED_COMMON_ENGLISH_WORDS = frozenset(
    {"the", "this", "you", "computer", "internet", "email", "github", "python", "rust", "windows"}
)
DEFAULT_PERF_MAX_P99_US = 2000
DEFAULT_PERF_INCREMENTAL_MAX_P99_US = 40000
PYTHON_RUNTIME = ".python-runtime"
RAPIDOCR_DIR = "RapidOCRLite"
RAPIDOCR_VENV = ".venv-rapidocr"
RAPIDOCR_PACKAGES = ".python-packages"
RAPIDOCR_HELPER = "kaixin_ocr_engine.py"
RAPIDOCR_HELPERS = (
    "kaixin_ocr_engine.py",
    "kaixin_ocr_policy.py",
    "kaixin_ocr_engine.cmd",
    "kaixin_cv_crop.py",
)
OPENCV_CROP_HELPER = "kaixin_cv_crop.py"
RAPIDOCR_REQUIRED_MODELS = (
    "PP-OCRv6_det_medium.onnx",
    "PP-OCRv6_det_small.onnx",
    "PP-OCRv6_rec_medium.onnx",
    "PP-OCRv6_rec_small.onnx",
    "ppocrv6_dict.txt",
)
PYTHON_PACKAGE_EXCLUDED_DIR_NAMES = frozenset(
    {
        "__pycache__",
        ".pytest_cache",
        ".mypy_cache",
        ".ruff_cache",
        "test",
        "tests",
        "testing",
        "benchmark",
        "benchmarks",
        "example",
        "examples",
        "pip",
        "setuptools",
    }
)
PYTHON_PACKAGE_EXCLUDED_FILE_SUFFIXES = frozenset(
    {".pyc", ".pyo", ".pdb", ".lib", ".exp", ".h", ".hpp", ".c", ".cpp", ".cxx", ".pxd", ".pyi"}
)
RUST_TARGET_BY_ARCH = {
    ARCH_X64: None,
    ARCH_X86: "i686-pc-windows-msvc",
}
PE_MACHINE_BY_ARCH = {
    ARCH_X64: 0x8664,  # IMAGE_FILE_MACHINE_AMD64
    ARCH_X86: 0x014C,  # IMAGE_FILE_MACHINE_I386
}


@dataclass(frozen=True)
class PackageVariant:
    id: str
    label: str
    stage_dir_name: str
    installer_suffix: str
    include_ocr: bool
    include_translation: bool


PACKAGE_VARIANTS: "OrderedDict[str, PackageVariant]" = OrderedDict(
    (
        (
            "ime",
            PackageVariant(
                id="ime",
                label="纯输入法",
                stage_dir_name="kaixin-package-ime",
                installer_suffix="ime",
                include_ocr=False,
                include_translation=True,
            ),
        ),
        (
            "ocr",
            PackageVariant(
                id="ocr",
                label="输入法+OCR 集合",
                stage_dir_name="kaixin-package-ocr",
                installer_suffix="ocr",
                include_ocr=True,
                include_translation=True,
            ),
        ),
    )
)

# User-facing aliases are normalized before package selection.  Keep the
# canonical ``ocr`` id and artifact names for compatibility with existing
# release automation while accepting the clearer ``full`` spelling.
PACKAGE_VARIANT_ALIASES: dict[str, str] = {
    "full": "ocr",
}

PACKAGE_BUILD_TOOL_RUST_ARTIFACTS = ("bake_lexicon.exe", "input_perf.exe", "input_eval.exe")
BASE_STAGED_RUST_ARTIFACTS = (
    "srf_ime_setup.exe",
    "srf_ime_settings.exe",
    "srf_ime_tray.exe",
    "srf_ime_engine.exe",
    "srf_ime_clipboard.exe",
    "srf_ime_clipboard_svc.exe",
    "srf_ime_handwrite.exe",
    "srf_ime_symbols.exe",
)
BASE_PACKAGE_RUST_ARTIFACTS = PACKAGE_BUILD_TOOL_RUST_ARTIFACTS + BASE_STAGED_RUST_ARTIFACTS
OPTIONAL_OCR_RUST_ARTIFACTS = ("srf_ime_ocr.exe",)
SMOKE_RUST_ARTIFACTS = ("input_smoke.exe", "input_eval.exe")
PERF_RUST_ARTIFACT = "input_perf.exe"
DEV_TOOL_RUST_BINS = {
    "lexicon_tool",
    "bake_lexicon",
    "build_ai_lexicons",
    "input_smoke",
    "input_eval",
    "input_perf",
    "phrase_len_eval",
    "learning_replay_eval",
    "build_blind_corpus",
}


def rust_artifact_names_for_variants(
    package_variants: list[PackageVariant] | None = None,
    include_smoke_tools: bool = True,
) -> list[str]:
    names = list(BASE_PACKAGE_RUST_ARTIFACTS)
    include_ocr = package_variants is None or any(
        variant.include_ocr for variant in package_variants
    )
    if include_ocr:
        names.extend(OPTIONAL_OCR_RUST_ARTIFACTS)
    if include_smoke_tools:
        names.extend(SMOKE_RUST_ARTIFACTS)
    return list(dict.fromkeys(names))


def staged_rust_exe_names(
    include_ocr: bool = True,
    include_translation: bool = True,
) -> list[str]:
    names = list(BASE_STAGED_RUST_ARTIFACTS)
    if include_ocr:
        names.extend(OPTIONAL_OCR_RUST_ARTIFACTS)
    return names


def rust_bin_names_for_artifacts(
    artifact_names: list[str] | tuple[str, ...],
) -> list[str]:
    """Convert packaged Rust exe artifact names to Cargo bin target names."""
    names: list[str] = []
    for artifact_name in artifact_names:
        path = Path(artifact_name)
        if path.suffix.lower() != ".exe":
            continue
        bin_name = path.stem
        if bin_name not in names:
            names.append(bin_name)
    return names


# Version
