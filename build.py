#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
开心输入法 - one-command packaging build script.

Usage:
    python build.py              # Full build (cargo + cmake + stage + Inno Setup)
    python build.py --no-inno    # Build but skip installer generation
    python build.py --quick      # Skip cargo/cmake; restage and package
    python build.py --clean      # Remove build caches before a fresh build
    python build.py --no-verify  # Skip Rust and C++ correctness verification
    python build.py --perf-smoke # Run the release input P99 performance gate
    python build.py --debug      # Build Debug artifacts
    python build.py --quiet      # Show only errors and final results
    python build.py --verbose    # Show complete child process output
    python build.py --dry-run    # Print commands without executing them
    python scripts/check_utf8_sources.py  # Verify source files are UTF-8

Artifacts:
    dist/kaixin-setup-ime-<version>-<YYYYMMDD-HHMMSS>.exe
    dist/kaixin-setup-ocr-<version>-<YYYYMMDD-HHMMSS>.exe
    dist/kaixin-package-ime/
    dist/kaixin-package-ocr/

If shared-engine Init timeouts occur at runtime, check:
%LOCALAPPDATA%\\kaixin\\logs\\engine.log
%LOCALAPPDATA%\\kaixin\\logs\\tsf.log
"""
from __future__ import annotations

import argparse
import bisect
import codecs
import configparser
import hashlib
import io
import json
import locale
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import zipfile
from collections import OrderedDict, deque
from concurrent.futures import Future, ThreadPoolExecutor
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from typing import Callable, TextIO

VERSION = "2.0.0"
VERSION_RE = re.compile(r"^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$")

# ---------------------------------------------------------------------------

def _read_version() -> str:
    """Build helper: _read_version."""
    version_file = Path(__file__).resolve().parent / "VERSION"
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
        ("rapidocr_payload", "RapidOCR-3.9.0/"),
    )
)
PREBAKED_LEXICON_MAGIC = b"SRFLX002"
PREBAKED_LEXICON_SCHEMA_VERSION = 9
PREBAKED_LEXICON_HEADER_SIZE = len(PREBAKED_LEXICON_MAGIC) + 4
PREBAKED_LEXICON_MIN_BYTES = 1000
COMMON_ENGLISH_LEXICON_RELATIVE_PATH = Path("en") / "kaixin_common_english.txt"
COMMON_ENGLISH_LICENSE_RELATIVE_PATH = Path("en") / "LICENSE.wordfreq.md"
EXPECTED_COMMON_ENGLISH_WORDS = 20_000
REQUIRED_COMMON_ENGLISH_WORDS = frozenset(
    {"the", "this", "you", "computer", "internet", "email", "github", "python", "rust", "windows"}
)
DEFAULT_PERF_MAX_P99_US = 2000
DEFAULT_PERF_INCREMENTAL_MAX_P99_US = 40000
PYTHON_RUNTIME = ".python-runtime"
RAPIDOCR_DIR = "RapidOCR-3.9.0"
RAPIDOCR_VENV = ".venv-rapidocr"
RAPIDOCR_PACKAGES = ".python-packages"
RAPIDOCR_HELPER = "kaixin_ocr_engine.py"
RAPIDOCR_HELPERS = ("kaixin_ocr_engine.py", "kaixin_ocr_engine.cmd", "kaixin_cv_crop.py")
OPENCV_CROP_HELPER = "kaixin_cv_crop.py"
RAPIDOCR_REQUIRED_MODELS = (
    "PP-OCRv6_det_medium.onnx",
    "PP-OCRv6_det_small.onnx",
    "PP-OCRv6_rec_medium.onnx",
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

PACKAGE_BUILD_TOOL_RUST_ARTIFACTS = ("bake_lexicon.exe",)
BASE_STAGED_RUST_ARTIFACTS = (
    "srf_ime_settings.exe",
    "srf_ime_tray.exe",
    "srf_ime_engine.exe",
    "srf_ime_clipboard.exe",
    "srf_ime_clipboard_svc.exe",
    "srf_ime_handwrite.exe",
)
BASE_PACKAGE_RUST_ARTIFACTS = PACKAGE_BUILD_TOOL_RUST_ARTIFACTS + BASE_STAGED_RUST_ARTIFACTS
OPTIONAL_OCR_RUST_ARTIFACTS = ("srf_ime_ocr.exe",)
SMOKE_RUST_ARTIFACTS = ("input_smoke.exe", "input_eval.exe")
PERF_RUST_ARTIFACT = "input_perf.exe"


def rust_artifact_names_for_variants(
    package_variants: list[PackageVariant] | None = None,
    include_smoke_tools: bool = True,
) -> list[str]:
    names = list(BASE_PACKAGE_RUST_ARTIFACTS)
    include_ocr = (
        package_variants is None
        or any(variant.include_ocr for variant in package_variants)
    )
    if include_ocr:
        names.extend(OPTIONAL_OCR_RUST_ARTIFACTS)
    if include_smoke_tools:
        names.extend(SMOKE_RUST_ARTIFACTS)
    return names


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

_COLOR_ENABLED = sys.platform == "win32" and os.environ.get("NO_COLOR") is None


def _init_color():
    """Build helper: _init_color."""
    try:
        import ctypes
        kernel32 = ctypes.windll.kernel32
        kernel32.SetConsoleCP(65001)
        kernel32.SetConsoleOutputCP(65001)
        if not _COLOR_ENABLED:
            return
        handle = kernel32.GetStdHandle(-11)  # STD_OUTPUT_HANDLE
        mode = ctypes.c_ulong()
        kernel32.GetConsoleMode(handle, ctypes.byref(mode))
        kernel32.SetConsoleMode(handle, mode.value | 0x4)  # ENABLE_VIRTUAL_TERMINAL_PROCESSING
    except Exception:
        pass


_init_color()


def _c(code: str, text: str) -> str:
    if not _COLOR_ENABLED:
        return text
    return f"\033[{code}m{text}\033[0m"


def green(t: str) -> str:  return _c("32", t)
def yellow(t: str) -> str: return _c("33", t)
def red(t: str) -> str:    return _c("31", t)
def cyan(t: str) -> str:   return _c("36", t)
def bold(t: str) -> str:   return _c("1", t)
def dim(t: str) -> str:    return _c("2", t)


# ---------------------------------------------------------------------------

class BuildContext:
    """Build helper: BuildContext."""
    def __init__(self):
        self.version: str = _read_version()
        self._log_lock = threading.Lock()
        self.reset_run_state()

    def reset_run_state(self) -> None:
        """Reset state that belongs to one invocation of :func:`main`.

        Keeping the context object global is useful for the small helper
        functions, but tests and embedders may invoke ``main`` more than once
        in a process.  Without resetting these fields, a previous run could
        leak its log handle, timings, signing warning, or artifact timestamp
        into the next build.
        """
        previous_log = getattr(self, "log_file", None)
        if previous_log is not None:
            try:
                previous_log.close()
            except OSError:
                pass
        self.quiet = False
        self.verbose = False
        self.dry_run = False
        self.no_version = False
        self.log_file = None
        self.step_results = OrderedDict()
        self.artifacts = []
        self.generator_cache_file = None
        self.sign_warning_emitted = False
        self.build_timestamp = datetime.now().strftime("%Y%m%d-%H%M%S")

    def log(self, msg: str) -> None:
        """Build helper: log."""
        if self.log_file:
            try:
                with self._log_lock:
                    self.log_file.write(msg.rstrip("\n") + "\n")
                    self.log_file.flush()
            except Exception:
                pass


ctx = BuildContext()


# Color output

def fmt_size(n: int) -> str:
    if n < 1024:
        return f"{n} B"
    if n < 1024 * 1024:
        return f"{n / 1024:.1f} KiB"
    return f"{n / (1024 * 1024):.2f} MiB"


def fmt_elapsed(seconds: float) -> str:
    m, s = divmod(int(seconds), 60)
    if m > 0:
        return f"{m}m {s}s"
    return f"{s}s"


def _cleanup_old_logs(dist_dir: Path, keep: int = 20) -> None:
    """Build helper: _cleanup_old_logs."""
    try:
        logs = sorted(
            [f for f in dist_dir.iterdir() if f.name.startswith("build-") and f.suffix == ".log"],
            key=lambda p: p.stat().st_mtime,
            reverse=True,
        )
        for old in logs[keep:]:
            try:
                old.unlink()
            except OSError:
                pass
    except OSError:
        pass


def _safe_unlink(path: Path) -> bool:
    """Build helper: _safe_unlink."""
    try:
        if path.is_file() or path.is_symlink():
            path.unlink(missing_ok=True)
            return True
    except OSError:
        return False
    return False


def purge_lexicon_caches(repo: Path, output_root: Path | None = None) -> list[Path]:
    """Build helper: purge_lexicon_caches."""
    deleted: list[Path] = []

    try:
        lex_dir = find_lexicon_dir(repo)
        for name in ("lexicon.bin", "hot_lexicon.bin"):
            p = lex_dir / name
            if p.exists() and (ctx.dry_run or _safe_unlink(p)):
                deleted.append(p)
    except FileNotFoundError:
        pass

    # ---------------------------------------------------------------------------
    if output_root and output_root.is_dir():
        # ---------------------------------------------------------------------------
        try:
            prebaked_bins = list(output_root.rglob("lexicon.bin"))
            prebaked_bins.extend(output_root.rglob("hot_lexicon.bin"))
            for p in prebaked_bins:
                if p.exists() and (ctx.dry_run or _safe_unlink(p)):
                    deleted.append(p)
        except OSError:
            pass

    return deleted


def sha256_prefix(path: Path, prefix_len: int = 8) -> str:
    """Build helper: sha256_prefix."""
    h = hashlib.sha256()
    try:
        with open(path, "rb") as f:
            while True:
                chunk = f.read(65536)
                if not chunk:
                    break
                h.update(chunk)
        return h.hexdigest()[:prefix_len]
    except Exception:
        return "?"


def sha256_file(path: Path) -> str:
    """Build helper: sha256_file."""
    h = hashlib.sha256()
    with open(path, "rb") as f:
        while True:
            chunk = f.read(65536)
            if not chunk:
                break
            h.update(chunk)
    return h.hexdigest()


def read_prebaked_lexicon_schema(path: Path) -> int:
    """Read and validate the fixed SRFLX002 header from a prebaked lexicon."""
    try:
        with path.open("rb") as f:
            header = f.read(PREBAKED_LEXICON_HEADER_SIZE)
    except OSError as exc:
        raise RuntimeError(f"cannot read prebaked lexicon header: {path}: {exc}") from exc
    if len(header) != PREBAKED_LEXICON_HEADER_SIZE:
        raise RuntimeError(f"prebaked lexicon header is truncated: {path}")
    if header[: len(PREBAKED_LEXICON_MAGIC)] != PREBAKED_LEXICON_MAGIC:
        raise RuntimeError(f"invalid prebaked lexicon magic (expected SRFLX002): {path}")
    return int.from_bytes(header[len(PREBAKED_LEXICON_MAGIC):], "little")


def verify_prebaked_lexicon(
    path: Path,
    expected_schema: int = PREBAKED_LEXICON_SCHEMA_VERSION,
) -> int:
    """Fail packaging when a missing, stale or obviously truncated lexicon is staged."""
    if not path.is_file():
        raise FileNotFoundError(f"missing staged prebaked lexicon: {path}")
    size = path.stat().st_size
    if size < PREBAKED_LEXICON_MIN_BYTES:
        raise RuntimeError(f"staged prebaked lexicon is unusually small ({size} bytes): {path}")
    schema = read_prebaked_lexicon_schema(path)
    if schema != expected_schema:
        raise RuntimeError(
            f"staged prebaked lexicon schema is v{schema}, expected v{expected_schema}: {path}; "
            "rebuild bake_lexicon.exe and restage without --quick/--skip-cargo"
        )
    return schema


def safe_filename_part(value: str) -> str:
    """Return a conservative filename component for release artifacts."""
    value = value.strip() or "unknown"
    return re.sub(r"[^0-9A-Za-z._-]+", "-", value).strip("-") or "unknown"


def timestamped_installer_name(
    version: str,
    timestamp: str,
    user_installer: bool = False,
    variant_suffix: str | None = None,
) -> str:
    prefix = "kaixin-user-setup" if user_installer else "kaixin-setup"
    if variant_suffix:
        prefix = f"{prefix}-{safe_filename_part(variant_suffix)}"
    return f"{prefix}-{safe_filename_part(version)}-{timestamp}.exe"


def stage_root_for_variant(repo: Path, variant: PackageVariant) -> Path:
    return repo / "dist" / variant.stage_dir_name


def parse_package_variant_ids(raw: str) -> list[str]:
    value = (raw or "all").strip().lower()
    if not value:
        return list(PACKAGE_VARIANTS.keys())
    requested = [part.strip().lower() for part in value.split(",") if part.strip()]
    if not requested:
        raise ValueError("no package variants selected")
    if "all" in requested:
        if len(requested) != 1:
            raise ValueError(
                "package variant 'all' cannot be combined with individual variants"
            )
        return list(PACKAGE_VARIANTS.keys())

    valid_ids = {*PACKAGE_VARIANTS.keys(), *PACKAGE_VARIANT_ALIASES.keys()}
    invalid = [variant_id for variant_id in requested if variant_id not in valid_ids]
    if invalid:
        valid = ", ".join(["all", *PACKAGE_VARIANTS.keys(), *PACKAGE_VARIANT_ALIASES.keys()])
        raise ValueError(f"unknown package variant(s): {', '.join(invalid)} (valid: {valid})")

    deduped: list[str] = []
    for requested_id in requested:
        variant_id = PACKAGE_VARIANT_ALIASES.get(requested_id, requested_id)
        if variant_id not in deduped:
            deduped.append(variant_id)
    return deduped


def resolve_package_variants(
    args: argparse.Namespace,
    ocr_ready: bool,
) -> tuple[list[PackageVariant], list[PackageVariant]]:
    requested_ids = parse_package_variant_ids(getattr(args, "package_variants", "all"))
    selected: list[PackageVariant] = []

    for variant_id in requested_ids:
        variant = PACKAGE_VARIANTS[variant_id]
        if variant.include_ocr and not ocr_ready:
            raise RuntimeError(
                f"package variant {variant.id!r} requires the local RapidOCR runtime; "
                f"refusing to silently downgrade the requested package set. "
                f"Create it with `python -m venv {RAPIDOCR_VENV}` and "
                f"`{RAPIDOCR_VENV}\\Scripts\\python.exe -m pip install "
                f"-r {RAPIDOCR_DIR}\\python\\requirements.txt onnxruntime`, "
                "then run `powershell -ExecutionPolicy Bypass -File "
                "scripts/fetch_rapidocr_models.ps1`; "
                f"or explicitly request `--package-variants ime`."
            )
        selected.append(variant)

    if not selected:
        raise RuntimeError("no package variants can be built with the current options")
    return selected, []


def cleanup_existing_installer_exes(dist_dir: Path) -> list[Path]:
    """Remove old top-level installer EXEs before writing the selected variants."""
    removed: list[Path] = []
    if not dist_dir.is_dir():
        return removed
    seen: set[Path] = set()
    for pattern in ("kaixin-setup*.exe", "kaixin-user-setup*.exe"):
        for path in dist_dir.glob(pattern):
            try:
                resolved = path.resolve()
            except OSError:
                resolved = path
            if resolved in seen:
                continue
            seen.add(resolved)
            if path.is_file() and _safe_unlink(path):
                removed.append(path)
    return removed


def git_release_info(repo: Path) -> dict:
    def _git(args: list[str]) -> str | None:
        try:
            return subprocess.check_output(
                ["git", *args],
                cwd=str(repo),
                text=True,
                stderr=subprocess.DEVNULL,
                encoding="utf-8",
                errors="replace",
            ).strip()
        except Exception:
            return None

    commit = _git(["rev-parse", "HEAD"])
    commit_short = _git(["rev-parse", "--short=12", "HEAD"])
    branch = _git(["rev-parse", "--abbrev-ref", "HEAD"])
    status = _git(["status", "--porcelain"])
    return {
        "commit": commit,
        "commit_short": commit_short,
        "branch": branch,
        "dirty": bool(status),
        "status_porcelain": status or "",
    }


def _git_state_fingerprint(repo: Path) -> tuple[float, str]:
    """Build helper: _git_state_fingerprint.

    Hash of (short HEAD commit, dirty flag) - the exact inputs both stamping
    sites embed (tsf-tip/CMakeLists.txt SRF_GIT_COMMIT/SRF_GIT_DIRTY,
    pinyin-ime/build.rs SRF_ENGINE_GIT_COMMIT/SRF_ENGINE_GIT_DIRTY).  The
    mtime contribution is 0.0 so mtime-based freshness logic is untouched; a
    bare commit or a dirty/clean transition changes the hash and forces both
    products to rebuild and re-stamp.  Degrades to the stamping fallback
    "unknown" when git is unavailable.
    """
    def _git(args: list[str]) -> str | None:
        try:
            return subprocess.check_output(
                ["git", *args],
                cwd=str(repo),
                text=True,
                stderr=subprocess.DEVNULL,
                encoding="utf-8",
                errors="replace",
            ).strip()
        except Exception:
            return None

    commit_short = _git(["rev-parse", "--short=12", "HEAD"]) or "unknown"
    dirty = bool(_git(["status", "--porcelain"]))
    h = hashlib.sha256()
    h.update(commit_short.encode("ascii", errors="replace"))
    h.update(b"\0")
    h.update(b"dirty" if dirty else b"clean")
    return 0.0, h.hexdigest()


def validate_version_sources(repo: Path) -> list[str]:
    errors: list[str] = []
    version_file = repo / "VERSION"
    if not version_file.is_file():
        errors.append(f"missing VERSION file: {version_file}")
        return errors

    version_text = version_file.read_text(encoding="utf-8", errors="replace").strip()
    if version_text != ctx.version:
        errors.append(f"VERSION mismatch: BuildContext={ctx.version!r}, VERSION file={version_text!r}")
    if VERSION != version_text:
        errors.append(f"build.py fallback VERSION={VERSION!r} does not match VERSION file={version_text!r}")

    for iss_name in ("kaixin.iss", "kaixin-user.iss"):
        iss = repo / "tsf-tip" / "installer" / iss_name
        if not iss.is_file():
            errors.append(f"missing Inno script: {iss}")
            continue
        text = iss.read_text(encoding="utf-8", errors="replace")
        if re.search(r'^\s*#define\s+KXAppVersion\s+"[^"]+"', text, flags=re.MULTILINE):
            errors.append(f"{iss_name} must not define a fallback KXAppVersion; pass /DKXAppVersion from build.py")
        if "KXAppVersion must be passed" not in text:
            errors.append(f"{iss_name} must fail fast when KXAppVersion is not supplied")
    return errors


def rust_target_dir(repo: Path, profile: str, arch: str = ARCH_X64) -> Path:
    target = RUST_TARGET_BY_ARCH.get(arch)
    base = repo / "pinyin-ime" / "target"
    return base / target / profile if target else base / profile


def cmake_build_dir_for(tsf_tip: Path, arch: str = ARCH_X64) -> Path:
    return tsf_tip / ("build-package" if arch == ARCH_X64 else f"build-package-{arch}")


def read_pe_machine(path: Path) -> int:
    """Read the PE machine field and reject truncated/non-PE inputs clearly."""
    try:
        with path.open("rb") as f:
            dos_header = f.read(64)
            if len(dos_header) < 64 or dos_header[:2] != b"MZ":
                raise RuntimeError(f"not a PE image (missing MZ header): {path}")
            pe_offset = int.from_bytes(dos_header[0x3C:0x40], "little")
            if pe_offset < 64:
                raise RuntimeError(f"invalid PE header offset: {path}")
            f.seek(pe_offset)
            signature = f.read(4)
            machine_bytes = f.read(2)
    except OSError as exc:
        raise FileNotFoundError(f"cannot read PE image: {path}: {exc}") from exc
    if signature != b"PE\0\0" or len(machine_bytes) != 2:
        raise RuntimeError(f"not a valid PE image: {path}")
    return int.from_bytes(machine_bytes, "little")


def assert_pe_arch(path: Path, arch: str, label: str) -> None:
    expected = PE_MACHINE_BY_ARCH[arch]
    machine = read_pe_machine(path)
    if machine != expected:
        raise RuntimeError(
            f"{label} is not {arch} (Machine=0x{machine:04X}, expected 0x{expected:04X}): {path}"
        )


def _rel_path(path: Path, root: Path) -> str:
    """Return a stable slash-separated path relative to root when possible."""
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return path.resolve().as_posix()


def _manifest_file_entry(path: Path, root: Path) -> dict:
    st = path.stat()
    return {
        "path": _rel_path(path, root),
        "size": st.st_size,
        "sha256": sha256_file(path),
        "mtime_utc": datetime.fromtimestamp(st.st_mtime, timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z"),
    }


def _manifest_relative_path(root: Path, rel: str) -> Path:
    if "\\" in rel:
        raise ValueError(f"unsafe package manifest path: {rel}")
    rel_path = PurePosixPath(rel)
    if rel_path.is_absolute() or any(part in ("", "..") or ":" in part for part in rel_path.parts):
        raise ValueError(f"unsafe package manifest path: {rel}")
    target = (root / Path(*rel_path.parts)).resolve()
    try:
        target.relative_to(root.resolve())
    except ValueError as exc:
        raise ValueError(f"package manifest path escapes package root: {rel}") from exc
    return target


def write_package_hash_manifest(
    root: Path,
    manifest_name: str = PACKAGE_HASH_MANIFEST,
) -> Path:
    """Write SHA-256 hashes for every staged package file except the manifest."""
    root = root.resolve()
    manifest_path = root / manifest_name
    lines: list[str] = []
    for path in sorted(root.rglob("*"), key=lambda p: p.as_posix().lower()):
        if not path.is_file():
            continue
        resolved = path.resolve()
        if resolved == manifest_path.resolve():
            continue
        rel = resolved.relative_to(root).as_posix()
        lines.append(f"{sha256_file(resolved)} *{rel}")
    manifest_path.write_text("\n".join(lines) + ("\n" if lines else ""), encoding="utf-8")
    return manifest_path


def write_component_manifest_from_package_manifest(
    root: Path,
    package_manifest: Path,
) -> Path:
    """Refresh component IDs from the final package hashes without rehashing payloads."""
    root = root.resolve()
    component_path = root / COMPONENT_MANIFEST
    package_lines = [
        line.strip()
        for line in package_manifest.read_text(encoding="utf-8-sig").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    parser = configparser.ConfigParser(interpolation=None)
    parser["components"] = {}
    for name, prefix in COMPONENT_PREFIXES.items():
        matching = [
            line
            for line in package_lines
            if " *" in line
            and line.split(" *", 1)[1].casefold().startswith(prefix.casefold())
        ]
        parser["components"][name] = (
            hashlib.sha256((("\n".join(matching)) + "\n").encode("utf-8")).hexdigest()
            if matching
            else "absent"
        )
    with component_path.open("w", encoding="utf-8", newline="\n") as handle:
        parser.write(handle, space_around_delimiters=False)

    component_rel = component_path.relative_to(root).as_posix()
    replacement = f"{sha256_file(component_path)} *{component_rel}"
    found = False
    for index, line in enumerate(package_lines):
        if " *" in line and line.split(" *", 1)[1].casefold() == component_rel.casefold():
            package_lines[index] = replacement
            found = True
            break
    if not found:
        package_lines.append(replacement)
    package_manifest.write_text("\n".join(package_lines) + "\n", encoding="utf-8")
    return component_path


def verify_package_hash_manifest(
    root: Path,
    manifest_name: str = PACKAGE_HASH_MANIFEST,
) -> int:
    """Validate a staged package SHA-256 manifest and return verified file count."""
    root = root.resolve()
    manifest_path = root / manifest_name
    if not manifest_path.is_file():
        raise FileNotFoundError(f"missing package integrity manifest: {manifest_path}")

    verified = 0
    for line_no, line in enumerate(manifest_path.read_text(encoding="utf-8-sig").splitlines(), 1):
        trimmed = line.strip()
        if not trimmed or trimmed.startswith("#"):
            continue
        match = re.fullmatch(r"([0-9a-fA-F]{64}) \*(.+)", trimmed)
        if not match:
            raise RuntimeError(f"invalid package manifest line {line_no}: {trimmed}")
        expected = match.group(1).lower()
        rel = match.group(2)
        path = _manifest_relative_path(root, rel)
        if not path.is_file():
            raise FileNotFoundError(f"package manifest entry missing file: {rel}")
        actual = sha256_file(path)
        if actual.lower() != expected:
            raise RuntimeError(f"package integrity check failed: {rel}")
        verified += 1

    if verified <= 0:
        raise RuntimeError(f"package integrity manifest is empty: {manifest_path}")
    return verified


def write_build_manifest(
    repo: Path,
    output_root: Path,
    profile: str,
    args: argparse.Namespace,
    artifacts: list[tuple[str, int, str]],
    extra_output_roots: list[Path] | None = None,
    copy_to_stage: bool = True,
    include_installer_files: bool = True,
    package_variant: PackageVariant | None = None,
) -> Path:
    """Write a machine-readable manifest for release audits and CI consumers."""
    files: list[dict] = []
    stage_roots = [output_root] + list(extra_output_roots or [])
    candidate_paths: list[Path] = []
    for stage_root in stage_roots:
        candidate_paths.extend([
            stage_root / "VERSION",
            stage_root / "LICENSE",
            stage_root / "LICENSE_SCOPE.md",
            stage_root / "NOTICE",
            stage_root / "THIRD_PARTY_NOTICES.md",
            stage_root / "current_runtime_payload.txt",
            *[stage_root / name for name in staged_rust_exe_names()],
            stage_root / "tools" / RAPIDOCR_HELPER,
            stage_root / "tools" / OPENCV_CROP_HELPER,
            packaged_python_runtime_path(stage_root),
            packaged_python_stdlib_marker(stage_root),
            rapidocr_packages_path(stage_root),
            stage_root / RAPIDOCR_DIR / "python" / "rapidocr" / "__init__.py",
            *[
                stage_root / RAPIDOCR_DIR / "python" / "rapidocr" / "models" / model
                for model in RAPIDOCR_REQUIRED_MODELS
            ],
            stage_root / "assets" / "kaixin-input.ico",
            stage_root / "lexicon" / "lexicon.bin",
            stage_root / "lexicon" / "hot_lexicon.bin",
        ])
    if args.portable_zip:
        candidate_paths.extend(stage_root.with_suffix(".zip") for stage_root in stage_roots)
    if include_installer_files and not args.no_inno and not args.debug:
        for installer_name in getattr(args, "installer_output_names", []):
            candidate_paths.append(repo / "dist" / installer_name)

    runtime_rel = ""
    for stage_root in stage_roots:
        marker = stage_root / "current_runtime_payload.txt"
        if marker.is_file():
            current_runtime_rel = marker.read_text(encoding="utf-8", errors="replace").strip()
            if stage_root == output_root:
                runtime_rel = current_runtime_rel
            runtime_dir = stage_root / current_runtime_rel
            for arch in BUILD_ARCHES:
                candidate_paths.append(runtime_dir / arch / "srf_tsf_tip.dll")
                candidate_paths.append(runtime_dir / arch / "srf_ime_overlay.exe")

    seen: set[Path] = set()
    for path in candidate_paths:
        try:
            resolved = path.resolve()
        except OSError:
            continue
        if resolved in seen or not path.is_file():
            continue
        seen.add(resolved)
        files.append(_manifest_file_entry(path, repo))

    def artifact_path(name: str) -> Path | None:
        for candidate in [
            repo / "dist" / name,
            *[stage_root / name for stage_root in stage_roots],
            rust_target_dir(repo, profile, ARCH_X64) / name,
        ]:
            if candidate.is_file():
                return candidate
        return None

    manifest = {
        "app": "开心输入法",
        "version": ctx.version,
        "profile": profile.capitalize(),
        "created_utc": datetime.now(timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z"),
        "git": git_release_info(repo),
        "runtime_payload": runtime_rel,
        "stage_roots": [
            _rel_path(stage_root, repo)
            for stage_root in stage_roots
            if stage_root.exists()
        ],
        "options": {
            "debug": bool(args.debug),
            "quick": bool(args.quick),
            "no_inno": bool(args.no_inno),
            "user_installer": bool(args.user_installer),
            "portable_zip": bool(args.portable_zip),
            "package_variant": package_variant.id if package_variant else None,
            "package_variants": list(getattr(args, "package_variant_ids", [])),
            "include_ocr": bool(package_variant.include_ocr) if package_variant else bool(getattr(args, "include_ocr_resolved", False)),
            "include_translation": bool(package_variant.include_translation) if package_variant else bool(getattr(args, "include_translation_resolved", False)),
            "skip_cargo": bool(args.skip_cargo),
            "skip_cmake": bool(args.skip_cmake),
            "verify": bool(args.verify),
            "perf_smoke": bool(args.perf_smoke),
            "perf_max_p99_us": int(args.perf_max_p99_us) if args.perf_smoke else None,
        },
        "artifacts": [
            {
                "name": name,
                "size": size,
                "sha256_prefix": sha,
                "sha256": sha256_file(path) if path else None,
            }
            for name, size, sha in artifacts
            for path in [artifact_path(name)]
        ],
        "model_sources": model_source_fingerprints(repo),
        "lexicon_sources": {
            "sha256": _lexicon_sources_fingerprint(repo)[1],
            "schema": read_prebaked_lexicon_schema(
                output_root / "lexicon" / "lexicon.bin"
            )
            if (output_root / "lexicon" / "lexicon.bin").is_file()
            else None,
            "hot_schema": read_prebaked_lexicon_schema(
                output_root / "lexicon" / "hot_lexicon.bin"
            )
            if (output_root / "lexicon" / "hot_lexicon.bin").is_file()
            else None,
            "staged_lexicon_bin": _manifest_file_entry(
                output_root / "lexicon" / "lexicon.bin",
                repo,
            )
            if (output_root / "lexicon" / "lexicon.bin").is_file()
            else None,
            "staged_hot_lexicon_bin": _manifest_file_entry(
                output_root / "lexicon" / "hot_lexicon.bin",
                repo,
            )
            if (output_root / "lexicon" / "hot_lexicon.bin").is_file()
            else None,
        },
        "files": files,
    }

    manifest_path = repo / "dist" / "build-manifest.json"
    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
    )
    if copy_to_stage:
        for stage_root in stage_roots:
            staged_manifest = stage_root / "build-manifest.json"
            if stage_root.is_dir():
                shutil.copy2(manifest_path, staged_manifest)
    return manifest_path


def print_msg(msg: str = "") -> None:
    """Build helper: print_msg."""
    if not ctx.quiet:
        print(msg)
    ctx.log(msg)


class StepTimer:
    """Build helper: StepTimer."""
    def __init__(self, title: str):
        self.title = title
        self._start = 0.0

    def __enter__(self):
        print_msg(f"\n{'=' * 60}")
        print_msg(bold(f"  {self.title}"))
        print_msg(f"{'=' * 60}")
        self._start = time.monotonic()
        return self

    def __exit__(self, *_):
        elapsed = time.monotonic() - self._start
        ctx.step_results[self.title] = elapsed
        print_msg(dim(f"  ({fmt_elapsed(elapsed)})"))


# ---------------------------------------------------------------------------

def _ensure_stdout_utf8() -> None:
    """Build helper: _ensure_stdout_utf8."""
    for stream_name in ("stdout", "stderr"):
        out = getattr(sys, stream_name, None)
        if out is None:
            continue
        try:
            if hasattr(out, "reconfigure"):
                out.reconfigure(encoding="utf-8", errors="replace")
        except (OSError, ValueError, TypeError, AttributeError):
            pass


def _write_stdout_text(text: str) -> None:
    """Build helper: _write_stdout_text."""
    try:
        sys.stdout.write(text)
        sys.stdout.flush()
    except UnicodeEncodeError:
        buf = getattr(sys.stdout, "buffer", None)
        if buf is not None:
            buf.write(text.encode("utf-8", errors="replace"))
            buf.flush()


def _decode_child_output(data: bytes) -> str:
    """Decode child-process output without corrupting Chinese paths from MSBuild/CMake."""
    preferred = locale.getpreferredencoding(False)
    encodings = ["utf-8-sig", "utf-8"]
    if preferred:
        encodings.append(preferred)
    encodings.extend(["gb18030", "mbcs"])

    seen: set[str] = set()
    for enc in encodings:
        key = enc.lower()
        if key in seen:
            continue
        seen.add(key)
        try:
            return data.decode(enc)
        except (LookupError, UnicodeDecodeError):
            continue
    return data.decode("utf-8", errors="replace")


def _redact_command(cmd: list[str], sensitive_values: tuple[str, ...]) -> list[str]:
    """Return a log-safe command while preserving the executable invocation."""
    if not sensitive_values:
        return list(cmd)
    secrets = {value for value in sensitive_values if value}
    return ["<redacted>" if part in secrets else part for part in cmd]


def _redact_text(text: str, sensitive_values: tuple[str, ...]) -> str:
    """Remove secret values from child output before it reaches logs."""
    redacted = text
    for value in sensitive_values:
        if value:
            redacted = redacted.replace(value, "<redacted>")
    return redacted


def run(cmd: list[str], cwd: Path, *, env: dict | None = None,
        capture_on_fail: int = 30,
        sensitive_values: tuple[str, ...] = ()) -> None:
    """Build helper: run."""
    # Match Windows command-line quoting so logged commands can be copied and
    # executed even when an executable or argument contains spaces.
    display_cmd = _redact_command(cmd, sensitive_values)
    display = subprocess.list2cmdline(display_cmd)
    print_msg(f"  {cyan('$')} {display}")

    if ctx.dry_run:
        return

    _ensure_stdout_utf8()

    merged_env = {**os.environ, **(env or {})}
    merged_env.setdefault("PYTHONUTF8", "1")
    merged_env.setdefault("PYTHONIOENCODING", "utf-8")

    # Keep the direct passthrough mode for ordinary commands, but capture
    # signing output so a tool that echoes its arguments cannot reveal a PFX
    # password even with ``--verbose``.
    if ctx.verbose and not sensitive_values:
        result = subprocess.run(cmd, cwd=str(cwd), env=merged_env)
        if result.returncode != 0:
            raise BuildCommandError(result.returncode, cmd, [], display_cmd=display_cmd)
        return

    proc = subprocess.Popen(
        cmd, cwd=str(cwd), env=merged_env,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )

    tail = deque(maxlen=max(1, capture_on_fail))

    def _reader():
        assert proc.stdout is not None
        for raw_line in proc.stdout:
            line = _redact_text(_decode_child_output(raw_line), sensitive_values)
            stripped = line.rstrip("\n\r")
            tail.append(stripped)
            if not ctx.quiet:
                _write_stdout_text(line)
            ctx.log(stripped)

    reader_thread = threading.Thread(target=_reader, daemon=True)
    reader_thread.start()
    proc.wait()
    reader_thread.join(timeout=5)

    if proc.returncode != 0:
        raise BuildCommandError(proc.returncode, cmd, list(tail), display_cmd=display_cmd)


class BuildCommandError(Exception):
    def __init__(
        self,
        returncode: int,
        cmd: list[str],
        tail_lines: list[str],
        *,
        display_cmd: list[str] | None = None,
    ):
        self.returncode = returncode
        self.cmd = cmd
        self.tail_lines = tail_lines
        self.display_cmd = display_cmd or cmd
        cmd_str = subprocess.list2cmdline(self.display_cmd)
        tail_str = "\n".join(f"    {l}" for l in tail_lines) if tail_lines else "    (no output)"
        super().__init__(f"command failed (exit {returncode}): {cmd_str}\n{tail_str}")


def run_parallel_tasks(
    tasks: list[tuple[str, Callable[[], None]]],
    *,
    context: str,
) -> list[tuple[str, Exception]]:
    """Run independent build tasks concurrently and collect their errors."""
    if not tasks:
        return []
    if len(tasks) == 1:
        try:
            tasks[0][1]()
            return []
        except Exception as exc:
            return [(tasks[0][0], exc)]

    errors: list[tuple[str, Exception]] = []
    err_lock = threading.Lock()
    fail_lock = threading.Lock()
    fail_notified = False
    task_names = [name for name, _ in tasks]

    def _report_first_fail(failed: str) -> None:
        nonlocal fail_notified
        with fail_lock:
            if fail_notified:
                return
            fail_notified = True
        others = ", ".join(name for name in task_names if name != failed) or "other tasks"
        if not ctx.quiet:
            print_msg(
                red(
                    f"! parallel {context}: {failed} failed first; {others} will continue to finish"
                    " (no interrupt signal sent)"
                )
            )

    def _run(name: str, fn: Callable[[], None]) -> None:
        try:
            fn()
        except Exception as exc:
            with err_lock:
                errors.append((name, exc))
            _report_first_fail(name)

    with ThreadPoolExecutor(max_workers=len(tasks)) as pool:
        futures: list[Future] = [pool.submit(_run, name, fn) for name, fn in tasks]
        for future in futures:
            future.result()

    with err_lock:
        return list(errors)


def which_or_die(name: str, hint: str = "") -> str:
    path = shutil.which(name)
    if path:
        return path
    msg = f"not found: {name}"
    if hint:
        msg += f". {hint}"
    raise FileNotFoundError(msg)


# Global state

def resolve_repo_root() -> Path:
    return Path(__file__).resolve().parent


def runtime_log_roots() -> list[Path]:
    roots: list[Path] = []
    local_app_data = os.environ.get("LOCALAPPDATA")
    if local_app_data:
        base = Path(local_app_data)
        roots.extend([
            base / "kaixin" / "logs",
            base / "kaixin",
        ])
    program_data = os.environ.get("PROGRAMDATA")
    if program_data:
        roots.append(Path(program_data) / "kaixin")

    unique: list[Path] = []
    seen: set[str] = set()
    for root in roots:
        key = str(root).lower()
        if key not in seen:
            seen.add(key)
            unique.append(root)
    return unique


def known_runtime_logs() -> list[str]:
    current_logs = [
        "tsf.log",
        "engine.log",
        "tray.log",
        "clipboard.log",
    ]
    rotated_logs: list[str] = []
    for name in current_logs:
        stem = name.rsplit(".", 1)[0]
        rotated_logs.append(f"{stem}.previous.log")
        rotated_logs.extend(f"{stem}.previous.{index}.log" for index in range(2, 6))

    return [
        *current_logs,
        *rotated_logs,
        "compatibility.log",
        "install_user.log",
        "install_machine.log",
        "install_language_list.log",
        "uninstall_user.log",
        "uninstall_machine.log",
        "register.log",
        "tip_registration.log",
        "tip_registration_user.log",
    ]


def collect_runtime_logs(repo: Path) -> list[Path]:
    dest_root = repo / "dist" / "runtime-logs"

    copied: list[Path] = []
    used_labels: set[str] = set()
    for source_root in runtime_log_roots():
        if not source_root.is_dir():
            continue
        label = source_root.name
        if label.lower() in used_labels:
            parent_label = re.sub(r"[^0-9A-Za-z_.-]+", "_", source_root.parent.name) or "root"
            label = f"{parent_label}-{source_root.name}"
        used_labels.add(label.lower())
        source_dest_root = dest_root / label
        if not ctx.dry_run:
            source_dest_root.mkdir(parents=True, exist_ok=True)

        for name in known_runtime_logs():
            src = source_root / name
            if not src.is_file():
                continue
            dest = source_dest_root / name
            if ctx.dry_run:
                copied.append(dest)
                continue
            shutil.copy2(src, dest)
            copied.append(dest)
    return copied


def find_lexicon_dir(repo: Path) -> Path:
    fixed = repo / "lexicon"
    if fixed.is_dir():
        return fixed.resolve()
    raise FileNotFoundError("lexicon directory not found: lexicon/")


def packaged_python_runtime_path(root: Path) -> Path:
    if sys.platform == "win32":
        return root / PYTHON_RUNTIME / "python.exe"
    return root / PYTHON_RUNTIME / "bin" / "python"


def packaged_python_stdlib_marker(root: Path) -> Path:
    return root / PYTHON_RUNTIME / "Lib" / "os.py"


def rapidocr_venv_python_path(root: Path) -> Path:
    if sys.platform == "win32":
        return root / RAPIDOCR_VENV / "Scripts" / "python.exe"
    return root / RAPIDOCR_VENV / "bin" / "python"


def rapidocr_packages_path(root: Path) -> Path:
    return root / RAPIDOCR_PACKAGES


def rapidocr_source_site_packages_path(repo: Path) -> Path:
    if sys.platform == "win32":
        return repo / RAPIDOCR_VENV / "Lib" / "site-packages"
    return repo / RAPIDOCR_VENV / "lib"


def rapidocr_runtime_status(repo: Path) -> tuple[bool, list[str]]:
    """Validate the OCR runtime before spending time on package staging.

    A copied venv can retain an absolute ``home=`` path from the machine that
    created it.  Detect that stale path early instead of failing in the
    PowerShell staging step after Rust/C++ compilation has already completed.
    """
    problems: list[str] = []
    tools_dir = repo / "tools"
    for helper in RAPIDOCR_HELPERS:
        path = tools_dir / helper
        if not path.is_file():
            problems.append(f"missing OCR helper: {path}")
        elif path.stat().st_size <= 0:
            problems.append(f"empty OCR helper: {path}")

    rapidocr_root = repo / RAPIDOCR_DIR / "python" / "rapidocr"
    if not (rapidocr_root / "__init__.py").is_file():
        problems.append(f"missing RapidOCR package: {rapidocr_root / '__init__.py'}")
    for model in RAPIDOCR_REQUIRED_MODELS:
        model_file = rapidocr_root / "models" / model
        if not model_file.is_file():
            problems.append(f"missing RapidOCR model: {model_file}")

    venv_dir = repo / RAPIDOCR_VENV
    cfg = venv_dir / "pyvenv.cfg"
    venv_python = rapidocr_venv_python_path(repo)
    site_packages = rapidocr_source_site_packages_path(repo)
    if not cfg.is_file():
        problems.append(f"missing OCR venv config: {cfg}")
    else:
        configured_home: Path | None = None
        try:
            for line in cfg.read_text(encoding="utf-8").splitlines():
                key, separator, value = line.partition("=")
                if separator and key.strip().lower() == "home":
                    configured_home = Path(value.strip().strip('"'))
                    break
        except OSError as e:
            problems.append(f"cannot read OCR venv config {cfg}: {e}")
        if configured_home is not None and not configured_home.is_dir():
            problems.append(
                f"OCR venv Python home does not exist: {configured_home}; "
                f"recreate {RAPIDOCR_VENV} with an installed Python"
            )
    if not venv_python.is_file():
        problems.append(f"missing OCR venv Python: {venv_python}")
    if not site_packages.is_dir():
        problems.append(f"missing OCR site-packages: {site_packages}")
    return not problems, problems


def find_iscc() -> Path | None:
    for p in [
        Path(r"C:\Program Files (x86)\Inno Setup 6\ISCC.exe"),
        Path(r"C:\Program Files\Inno Setup 6\ISCC.exe"),
    ]:
        if p.is_file():
            return p
    w = shutil.which("ISCC.exe")
    return Path(w).resolve() if w else None


def signing_requested(args: argparse.Namespace) -> bool:
    return bool(
        args.sign
        or args.sign_required
        or os.environ.get("KX_SIGN_CERT_SHA1")
        or os.environ.get("KX_SIGN_PFX")
    )


def find_signtool() -> Path | None:
    configured = os.environ.get("KX_SIGNTOOL")
    if configured:
        p = Path(configured)
        if p.is_file():
            return p.resolve()
    w = shutil.which("signtool.exe") or shutil.which("signtool")
    if w:
        return Path(w).resolve()
    roots = [
        Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Windows Kits" / "10" / "bin",
        Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Windows Kits" / "10" / "bin",
    ]
    for root in roots:
        if not root.is_dir():
            continue
        for candidate in sorted(root.glob(r"*\x64\signtool.exe"), reverse=True):
            if candidate.is_file():
                return candidate.resolve()
    return None


def sign_file(path: Path, args: argparse.Namespace) -> bool:
    if not signing_requested(args):
        return False
    if ctx.dry_run:
        print_msg(f"  [dry-run] sign: {path}")
        return True
    if not path.is_file():
        raise FileNotFoundError(f"cannot sign missing file: {path}")

    signtool = find_signtool()
    thumbprint = os.environ.get("KX_SIGN_CERT_SHA1", "").strip()
    pfx = os.environ.get("KX_SIGN_PFX", "").strip()
    pfx_password = os.environ.get("KX_SIGN_PFX_PASSWORD", "")
    timestamp = os.environ.get("KX_SIGN_TIMESTAMP_URL", "http://timestamp.digicert.com").strip()

    missing_reason = None
    if not signtool:
        missing_reason = "signtool.exe not found"
    elif not thumbprint and not pfx:
        missing_reason = "set KX_SIGN_CERT_SHA1 or KX_SIGN_PFX"
    if missing_reason:
        if args.sign_required:
            raise RuntimeError(f"code signing requested but unavailable: {missing_reason}")
        if not ctx.sign_warning_emitted:
            print_msg(f"  {yellow('!')} code signing skipped: {missing_reason}")
            ctx.sign_warning_emitted = True
        return False

    cmd = [str(signtool), "sign", "/fd", "SHA256"]
    if timestamp:
        cmd += ["/tr", timestamp, "/td", "SHA256"]
    if thumbprint:
        cmd += ["/sha1", thumbprint]
    else:
        cmd += ["/f", pfx]
        if pfx_password:
            cmd += ["/p", pfx_password]
    cmd.append(str(path))
    # Never print the PFX password in the command transcript or failure text.
    # The raw command is still passed to signtool, while ``run`` uses the
    # redacted representation for stdout, build logs, and BuildCommandError.
    run(cmd, cwd=path.parent, sensitive_values=(pfx_password,))
    return True


def sign_staged_binaries(output_root: Path, args: argparse.Namespace) -> list[Path]:
    signed: list[Path] = []
    marker = output_root / "current_runtime_payload.txt"
    runtime_rel = marker.read_text(encoding="utf-8", errors="replace").strip() if marker.is_file() else ""
    candidates = [output_root / name for name in staged_rust_exe_names()]
    if runtime_rel:
        runtime_dir = output_root / runtime_rel
        candidates.extend([runtime_dir / arch / "srf_tsf_tip.dll" for arch in BUILD_ARCHES])
        candidates.extend([runtime_dir / arch / "srf_ime_overlay.exe" for arch in BUILD_ARCHES])
    for path in candidates:
        if path.is_file() and sign_file(path, args):
            signed.append(path)
    return signed


def find_powershell() -> Path:
    for name in ["powershell.exe", "pwsh.exe"]:
        w = shutil.which(name)
        if w:
            return Path(w).resolve()
    raise FileNotFoundError("PowerShell not found")


def detect_vs_generators(cmake: str, arch: str = ARCH_X64) -> list[tuple[str, list[str]]]:
    """Build helper: detect_vs_generators."""
    platform = "Win32" if arch == ARCH_X86 else "x64"
    try:
        text = subprocess.check_output(
            [cmake, "--help"], stderr=subprocess.STDOUT,
            text=True, encoding="utf-8", errors="ignore",
        )
        found: list[tuple[int, str]] = []
        for line in text.splitlines():
            m = re.match(r"^\s*\*?\s*(Visual Studio\s+(\d+)\s+\d{4})\s*=", line)
            if m:
                found.append((int(m.group(2)), m.group(1).strip()))
        if found:
            found.sort(key=lambda x: x[0], reverse=True)
            return [(name, ["-A", platform]) for _, name in found]
    except Exception:
        pass
    return [
        ("Visual Studio 18 2026", ["-A", platform]),
        ("Visual Studio 17 2022", ["-A", platform]),
        ("Visual Studio 16 2019", ["-A", platform]),
    ]


def find_tsf_runtime_pair(
    tsf_tip: Path, build_dir: Path, profile: str
) -> tuple[Path, Path]:
    """Find a TIP DLL and overlay executable produced by the same build directory."""
    candidates: list[tuple[Path, Path]] = []
    dirs = [build_dir]
    if build_dir.resolve() == (tsf_tip / "build-package").resolve():
        dirs.extend([tsf_tip / "build-codex", tsf_tip / "build"])
    for directory in dirs:
        candidates.append(
            (
                directory / profile / "srf_tsf_tip.dll",
                directory / profile / "srf_ime_overlay.exe",
            )
        )
        candidates.append(
            (
                directory / "srf_tsf_tip.dll",
                directory / "srf_ime_overlay.exe",
            )
        )
    for dll, overlay in candidates:
        if dll.is_file() and overlay.is_file():
            return dll.resolve(), overlay.resolve()
    raise FileNotFoundError(
        "Could not find a co-located srf_tsf_tip.dll + srf_ime_overlay.exe pair; "
        "make sure the CMake build succeeded.\nChecked:\n"
        + "\n".join(f"  {dll}\n  {overlay}" for dll, overlay in candidates)
    )


def find_tsf_dll(tsf_tip: Path, build_dir: Path, profile: str) -> Path:
    """Build helper: find_tsf_dll."""
    return find_tsf_runtime_pair(tsf_tip, build_dir, profile)[0]


def find_overlay_exe(tsf_tip: Path, build_dir: Path, profile: str) -> Path:
    """Find the independent candidate-overlay executable for one architecture."""
    return find_tsf_runtime_pair(tsf_tip, build_dir, profile)[1]


# Utilities

def load_cached_generator(cache_file: Path, default_arch: str = ARCH_X64) -> tuple[str, list[str]] | None:
    """Build helper: load_cached_generator."""
    if not cache_file.is_file():
        return None
    try:
        text = cache_file.read_text(encoding="utf-8").strip()
        if not text:
            return None
        parts = text.split("|")
        gen_name = parts[0]
        platform = "Win32" if default_arch == ARCH_X86 else "x64"
        gen_args = parts[1:] if len(parts) > 1 else ["-A", platform]
        return (gen_name, gen_args)
    except Exception:
        return None


def save_cached_generator(cache_file: Path, gen_name: str, gen_args: list[str]) -> None:
    """Build helper: save_cached_generator."""
    try:
        cache_file.parent.mkdir(parents=True, exist_ok=True)
        cache_file.write_text("|".join([gen_name] + gen_args), encoding="utf-8")
    except Exception:
        pass


# ---------------------------------------------------------------------------


def _tree_source_stats(root: Path, extensions: set[str]) -> tuple[float, str]:
    """Build helper: _tree_source_stats."""
    if not root.is_dir():
        return 0.0, hashlib.sha256().hexdigest()
    paths: list[Path] = []
    try:
        for p in root.rglob("*"):
            if p.is_file() and p.suffix.lower() in extensions:
                paths.append(p)
    except OSError:
        pass
    paths.sort(key=lambda x: x.as_posix().lower())
    h = hashlib.sha256()
    max_m = 0.0
    root_abs = root.resolve()
    for p in paths:
        try:
            st = p.stat()
            max_m = max(max_m, st.st_mtime)
            rel = os.path.relpath(str(p), str(root_abs)).replace("\\", "/")
            h.update(rel.encode("utf-8", errors="surrogateescape"))
            h.update(b"\0")
            h.update(str(int(st.st_mtime_ns)).encode("ascii"))
            h.update(b"\n")
        except OSError:
            pass
    return max_m, h.hexdigest()


def _tree_source_stats_filtered(
    root: Path, excluded_dir_names: frozenset[str] | set[str]
) -> tuple[float, str]:
    """Fingerprint every source file below ``root``, excluding generated trees."""
    if not root.is_dir():
        return 0.0, hashlib.sha256().hexdigest()
    excluded = {name.casefold() for name in excluded_dir_names}
    paths: list[Path] = []
    try:
        for current_root, dir_names, file_names in os.walk(root):
            dir_names[:] = [
                name for name in dir_names if name.casefold() not in excluded
            ]
            current = Path(current_root)
            paths.extend(current / name for name in file_names)
    except OSError:
        pass
    paths.sort(key=lambda path: path.as_posix().casefold())
    digest = hashlib.sha256()
    max_mtime = 0.0
    root_abs = root.resolve()
    for path in paths:
        try:
            stat = path.stat()
            max_mtime = max(max_mtime, stat.st_mtime)
            relative = os.path.relpath(str(path), str(root_abs)).replace("\\", "/")
            digest.update(relative.encode("utf-8", errors="surrogateescape"))
            digest.update(b"\0")
            digest.update(str(int(stat.st_mtime_ns)).encode("ascii"))
            digest.update(b"\0")
            digest.update(str(int(stat.st_size)).encode("ascii"))
            digest.update(b"\n")
        except OSError:
            pass
    return max_mtime, digest.hexdigest()


def _source_file_stats(path: Path, repo: Path) -> tuple[float, str]:
    """Build helper: _source_file_stats."""
    h = hashlib.sha256()
    try:
        st = path.stat()
        rel = _rel_path(path, repo)
        h.update(rel.encode("utf-8", errors="surrogateescape"))
        h.update(b"\0")
        h.update(str(int(st.st_mtime_ns)).encode("ascii"))
        h.update(b"\0")
        h.update(str(int(st.st_size)).encode("ascii"))
        h.update(b"\n")
        return st.st_mtime, h.hexdigest()
    except OSError:
        return 0.0, h.hexdigest()


def _read_sig(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8").strip()
    except OSError:
        return ""


def _write_sig(path: Path, content: str) -> None:
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content + "\n", encoding="utf-8")
    except OSError:
        pass


def _cargo_inputs_fingerprint(repo: Path, profile: str, arch: str = ARCH_X64) -> tuple[float, str, Path] | None:
    """Build helper: _cargo_inputs_fingerprint."""
    target_dir = rust_target_dir(repo, profile, arch)
    src_dir = repo / "pinyin-ime" / "src"
    data_dir = repo / "pinyin-ime" / "data"
    cargo_toml = repo / "pinyin-ime" / "Cargo.toml"
    cargo_lock = repo / "pinyin-ime" / "Cargo.lock"
    build_rs = repo / "pinyin-ime" / "build.rs"
    app_icon = repo / "assets" / "kaixin-input.ico"
    if not src_dir.is_dir() or not cargo_toml.is_file():
        return None
    max_rs, fp_rs = _tree_source_stats(src_dir, {".rs"})
    max_data, fp_data = _tree_source_stats(data_dir, {".txt", ".bin", ".yaml", ".yml", ".json"})
    max_lex, fp_lex = _lexicon_sources_fingerprint(repo)
    try:
        st = cargo_toml.stat()
        st_build = build_rs.stat() if build_rs.is_file() else None
        max_lock, fp_lock = _source_file_stats(cargo_lock, repo)
        max_icon, fp_icon = _source_file_stats(app_icon, repo)
    except OSError:
        return None
    _, fp_git = _git_state_fingerprint(repo)
    max_src = max(
        max_rs,
        max_data,
        max_lex,
        st.st_mtime,
        (st_build.st_mtime if st_build is not None else 0.0),
        max_lock,
        max_icon,
    )
    comb = hashlib.sha256()
    comb.update(fp_rs.encode("ascii"))
    comb.update(b"\0")
    comb.update(fp_data.encode("ascii"))
    comb.update(b"\0")
    comb.update(fp_lex.encode("ascii"))
    comb.update(b"\0")
    comb.update(str(int(st.st_mtime_ns)).encode("ascii"))
    if st_build is not None:
        comb.update(b"\0")
        comb.update(str(int(st_build.st_mtime_ns)).encode("ascii"))
    comb.update(b"\0")
    comb.update(fp_lock.encode("ascii"))
    comb.update(b"\0")
    comb.update(fp_icon.encode("ascii"))
    comb.update(b"\0")
    comb.update(fp_git.encode("ascii"))
    sig_path = target_dir / ".cargo_source_manifest.sha256"
    return max_src, comb.hexdigest(), sig_path


def _refresh_cargo_build_sig(repo: Path, profile: str, arch: str = ARCH_X64) -> None:
    if ctx.dry_run:
        return
    idt = _cargo_inputs_fingerprint(repo, profile, arch)
    if idt is None:
        return
    _, comb_fp, sig_path = idt
    _write_sig(sig_path, comb_fp)


def is_cargo_up_to_date(
    repo: Path,
    profile: str,
    arch: str = ARCH_X64,
    required_artifacts: list[str] | tuple[str, ...] | None = None,
) -> bool:
    """Build helper: is_cargo_up_to_date."""
    target_dir = rust_target_dir(repo, profile, arch)
    artifact_names = (
        required_artifacts
        if required_artifacts is not None
        else rust_artifact_names_for_variants()
    )
    required = [target_dir / name for name in artifact_names]
    if not all(p.is_file() for p in required):
        return False
    idt = _cargo_inputs_fingerprint(repo, profile, arch)
    if idt is None:
        return False
    max_src, comb_fp, sig_path = idt
    try:
        min_art = min(p.stat().st_mtime for p in required)
    except OSError:
        return False
    if min_art < max_src:
        return False
    return _read_sig(sig_path) == comb_fp


def _cmake_inputs_fingerprint(tsf_tip: Path, repo: Path | None = None) -> tuple[float, str] | None:
    src_dir = tsf_tip / "src"
    inc_dir = tsf_tip / "include"
    tests_dir = tsf_tip / "tests"
    cm = tsf_tip / "CMakeLists.txt"
    module_def = tsf_tip / "srf_tsf_tip.def"
    version_file = (repo or tsf_tip.parent) / "VERSION"
    if not src_dir.is_dir() or not inc_dir.is_dir() or not cm.is_file():
        return None
    max_cpp, fp_cpp = _tree_source_stats(
        src_dir, {".cpp", ".c", ".ipp", ".rc", ".manifest"}
    )
    max_h, fp_h = _tree_source_stats(inc_dir, {".h"})
    max_tests, fp_tests = _tree_source_stats(tests_dir, {".cpp", ".c", ".h"})
    try:
        st = cm.stat()
        max_def, fp_def = _source_file_stats(module_def, repo or tsf_tip)
        max_version, fp_version = _source_file_stats(version_file, repo or tsf_tip.parent)
    except OSError:
        return None
    _, fp_git = _git_state_fingerprint(repo or tsf_tip.parent)
    max_src = max(max_cpp, max_h, max_tests, st.st_mtime, max_def, max_version)
    comb = hashlib.sha256()
    comb.update(fp_cpp.encode("ascii"))
    comb.update(b"\0")
    comb.update(fp_h.encode("ascii"))
    comb.update(b"\0")
    comb.update(fp_tests.encode("ascii"))
    comb.update(b"\0")
    comb.update(str(int(st.st_mtime_ns)).encode("ascii"))
    comb.update(b"\0")
    comb.update(fp_def.encode("ascii"))
    comb.update(b"\0")
    comb.update(fp_version.encode("ascii"))
    comb.update(b"\0")
    comb.update(fp_git.encode("ascii"))
    return max_src, comb.hexdigest()


def _refresh_cmake_build_sig(tsf_tip: Path, build_dir: Path, profile: str) -> None:
    if ctx.dry_run:
        return
    profile_cap = profile.capitalize()
    try:
        dll, _ = find_tsf_runtime_pair(tsf_tip, build_dir, profile_cap)
    except FileNotFoundError:
        return
    # ---------------------------------------------------------------------------
    fp = _cmake_inputs_fingerprint(tsf_tip, tsf_tip.parent)
    if fp is None:
        return
    _, comb_fp = fp
    _write_sig(dll.parent / ".cmake_source_manifest.sha256", comb_fp)


def is_cmake_up_to_date(repo: Path, tsf_tip: Path, profile: str, arch: str = ARCH_X64) -> bool:
    """Build helper: is_cmake_up_to_date."""
    profile_cap = profile.capitalize()
    build_dir = cmake_build_dir_for(tsf_tip, arch)
    try:
        dll, overlay = find_tsf_runtime_pair(tsf_tip, build_dir, profile_cap)
    except FileNotFoundError:
        return False
    fp = _cmake_inputs_fingerprint(tsf_tip, repo)
    if fp is None:
        return False
    max_src, comb_fp = fp
    sig_path = dll.parent / ".cmake_source_manifest.sha256"
    try:
        # Individual MSBuild targets are incremental: an overlay-only source
        # change need not relink the TIP DLL. The signature file is refreshed
        # only after the complete CMake build succeeds, so it is the correct
        # completion timestamp for the runtime pair.
        build_completed_mtime = sig_path.stat().st_mtime
    except OSError:
        return False
    if build_completed_mtime < max_src:
        return False
    return _read_sig(sig_path) == comb_fp


# Subprocess runner

def _looks_utf16(data: bytes, le: bool) -> bool:
    if len(data) < 4 or len(data) % 2:
        return False
    nul_idx = 1 if le else 0
    nul = sum(1 for i in range(0, len(data), 2) if data[i + nul_idx] == 0)
    return nul > 0 and nul * 3 >= len(data) // 2


def _has_cjk(s: str) -> bool:
    return any(
        0x3400 <= ord(c) <= 0x4DBF or 0x4E00 <= ord(c) <= 0x9FFF
        or 0xF900 <= ord(c) <= 0xFAFF or 0x20000 <= ord(c) <= 0x2EBEF
        for c in s
    )


def _weight_repeats(w: int) -> int:
    # Lexicon frequencies are normalized to the 1..10000 score space.
    # Keep five coarse corpus-repeat buckets without relying on the old
    # million-scale source weights.
    if w >= 7_500: return 5
    if w >= 5_000: return 4
    if w >= 2_500: return 3
    if w >= 100:   return 2
    return 1


def _adjust_weight_for_source(term: str, raw_weight: int, source_file_name: str) -> int:
    """Sanitize source weights before frequency calibration."""
    _ = (term, source_file_name)
    return max(1, raw_weight)


def _parse_lexicon_weight(parts: list[str]) -> int:
    for field in reversed(parts[1:]):
        try:
            return int(field.strip())
        except ValueError:
            continue
    return 1


def _is_native_lexicon_text(path: Path) -> bool:
    return path.suffix.lower() == ".txt" and not path.name.lower().startswith("readme")


LEXICON_LAYER_ORDER = {"core": 0, "base": 1, "ext": 2, "large": 3, "en": 4}


def _lexicon_layer(path: Path) -> str | None:
    parts = {part.casefold() for part in path.parts}
    if "zh-ext" in parts:
        return "ext"
    for legacy_layer in ("core", "base", "ext", "large", "en"):
        if legacy_layer in parts:
            return legacy_layer
    if "zh" not in parts:
        return None
    name = path.name.casefold()
    if name in {
        "kaixin_explicit.txt",
        "kaixin_polyphone.txt",
        "kaixin_pronunciation_aliases.txt",
    }:
        return "core"
    if "_tail_" in name or name.startswith("large_"):
        return "large"
    return "base"


def _lexicon_sort_key(path: Path, _root: Path) -> tuple[int, str]:
    layer = LEXICON_LAYER_ORDER.get(_lexicon_layer(path), 99)
    return layer, path.as_posix().lower()


def _is_default_enabled_lexicon_source(path: Path) -> bool:
    """Keep build-time LM inputs aligned with the runtime default lexicon set."""
    name = path.name.lower()
    return not name.endswith("_纯名单.txt")


def _lexicon_term_char_count(term: str) -> int:
    return sum(not ch.isspace() for ch in term)


def _is_frequency_reference_lexicon_source(path: Path) -> bool:
    return _lexicon_layer(path) == "base"


def _is_absolute_priority_lexicon_source(path: Path) -> bool:
    return _lexicon_layer(path) == "core" and path.name.lower() in {
        "kaixin_polyphone.txt",
        "kaixin_pronunciation_aliases.txt",
    }


def _is_unscaled_lexicon_source(path: Path) -> bool:
    """Keep intentionally low-priority sources on their authored scale.

    Names and local places contain flat low-priority weights. Percentile
    calibration would turn those values into an arbitrary point in the base
    corpus distribution, making recall-only entries look common.
    """
    return _is_absolute_priority_lexicon_source(path) or path.name.lower() in {
        "people_names.txt",
        "hangzhou_local.txt",
    }


def _source_frequency_values(path: Path) -> dict[int, list[int]]:
    """Read usable CJK entry weights grouped by phrase length from one source."""
    values: dict[int, list[int]] = {}
    for line in _iter_lexicon_lines(path):
        line = line.strip().lstrip("\ufeff")
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        term = parts[0].strip()
        if not term or not _has_cjk(term):
            continue
        values.setdefault(_lexicon_term_char_count(term), []).append(
            _adjust_weight_for_source(term, _parse_lexicon_weight(parts), path.name)
        )
    return values


def _build_frequency_calibrators(
    source_files: list[Path],
) -> dict[Path, tuple[dict[int, list[int]], dict[int, list[int]]]]:
    """Map auxiliary source percentiles onto the base lexicon's frequency scale."""
    reference: dict[int, list[int]] = {}
    source_values: dict[Path, dict[int, list[int]]] = {}
    for path in source_files:
        values = _source_frequency_values(path)
        if _is_frequency_reference_lexicon_source(path):
            for length, weights in values.items():
                reference.setdefault(length, []).extend(weights)
        elif not _is_unscaled_lexicon_source(path):
            source_values[path] = values
    for weights in reference.values():
        weights.sort()
    for values in source_values.values():
        for weights in values.values():
            weights.sort()
    return {path: (values, reference) for path, values in source_values.items() if values}


def _calibrated_source_weight(
    term: str,
    raw_weight: int,
    calibrator: tuple[dict[int, list[int]], dict[int, list[int]]] | None,
) -> int:
    """Map auxiliary source percentiles onto the base lexicon's frequency scale.

    This is the Python mirror of ``SourceFrequencyCalibrator::normalize`` in
    ``pinyin-ime/src/thuocl.rs``.  The two implementations MUST stay in sync:
    any change to the percentile-mapping logic here must be reflected in the
    Rust side, and vice versa.  See the Rust doc comment for rationale.
    """
    if calibrator is None:
        return max(1, raw_weight)
    source, reference = calibrator
    length = _lexicon_term_char_count(term)
    source_weights = source.get(length)
    reference_weights = reference.get(length)
    if not source_weights or not reference_weights:
        return max(1, raw_weight)
    lower = bisect.bisect_left(source_weights, raw_weight)
    upper = bisect.bisect_right(source_weights, raw_weight)
    rank = lower + max(0, upper - lower - 1) // 2
    rank = min(rank, len(source_weights) - 1)
    if len(source_weights) <= 1 or len(reference_weights) <= 1:
        return max(1, reference_weights[0])
    index = rank * (len(reference_weights) - 1) // (len(source_weights) - 1)
    index = min(index, len(reference_weights) - 1)
    return max(1, reference_weights[index])


def _lexicon_source_files(repo: Path) -> list[Path]:
    lex_dir = find_lexicon_dir(repo)
    return sorted(
        [
            f
            for f in lex_dir.rglob("*")
            if f.is_file()
            and _is_default_enabled_lexicon_source(f)
            and _is_native_lexicon_text(f)
        ],
        key=lambda p: _lexicon_sort_key(p, lex_dir),
    )


def _corpus_lexicon_source_files(repo: Path) -> list[Path]:
    """Return only sources that can contribute CJK terms to corpus.txt."""
    lex_dir = find_lexicon_dir(repo)
    return [
        path
        for path in _lexicon_source_files(repo)
        if "en" not in {part.casefold() for part in path.relative_to(lex_dir).parts}
    ]


def _source_files_fingerprint(lex_dir: Path, source_files: list[Path]) -> tuple[float, str]:
    h = hashlib.sha256()
    max_m = 0.0
    for path in source_files:
        try:
            st = path.stat()
            max_m = max(max_m, st.st_mtime)
            rel = path.resolve().relative_to(lex_dir.resolve()).as_posix()
            h.update(rel.encode("utf-8", errors="surrogateescape"))
            h.update(b"\0")
            h.update(sha256_file(path).encode("ascii"))
            h.update(b"\n")
        except (OSError, ValueError):
            continue
    return max_m, h.hexdigest()


def _lexicon_sources_fingerprint(repo: Path) -> tuple[float, str]:
    try:
        lex_dir = find_lexicon_dir(repo)
        return _source_files_fingerprint(lex_dir, _lexicon_source_files(repo))
    except FileNotFoundError:
        return 0.0, hashlib.sha256().hexdigest()


def _corpus_sources_fingerprint(repo: Path) -> tuple[float, str]:
    try:
        lex_dir = find_lexicon_dir(repo)
        return _source_files_fingerprint(lex_dir, _corpus_lexicon_source_files(repo))
    except FileNotFoundError:
        return 0.0, hashlib.sha256().hexdigest()


def _corpus_fingerprint_path(corpus: Path) -> Path:
    return corpus.with_name(corpus.name + ".source.sha256")


def model_source_fingerprints(repo: Path) -> dict:
    """Return source hashes that determine the embedded Rust input model."""
    data_dir = repo / "pinyin-ime" / "data"
    inputs = [
        data_dir / "pinyin_supported_chars.txt",
        data_dir / "syllables.txt",
        data_dir / "corpus.txt",
        _corpus_fingerprint_path(data_dir / "corpus.txt"),
        repo / "pinyin-ime" / "build.rs",
    ]
    files = []
    h = hashlib.sha256()
    for path in inputs:
        if not path.is_file():
            continue
        entry = _manifest_file_entry(path, repo)
        files.append(entry)
        h.update(entry["path"].encode("utf-8"))
        h.update(b"\0")
        h.update(entry["sha256"].encode("ascii"))
        h.update(b"\n")
    return {"sha256": h.hexdigest(), "files": files}


def require_fresh_rust_artifacts(
    repo: Path,
    profile: str,
    reason: str,
    required_artifacts: list[str] | tuple[str, ...] | None = None,
) -> None:
    """Fail packaging if embedded model inputs changed after the Rust build."""
    if is_cargo_up_to_date(
        repo,
        profile,
        ARCH_X64,
        required_artifacts=required_artifacts,
    ):
        return
    raise RuntimeError(
        f"Rust artifacts are stale for {reason}; rebuild without --quick/--skip-cargo "
        "so pinyin_supported_chars.txt, syllables.txt, corpus.txt and lexicon-derived model data are re-embedded. "
        "For an official package, rerun a full build without --skip-cargo."
    )


def print_skip_cargo_packaging_warning() -> None:
    print_msg(f"\n  {yellow('!')} Rust build was skipped (--skip-cargo).")
    print_msg(
        "    Staged files may reuse old Rust binaries; official packaging must be rerun "
        "without --skip-cargo."
    )


def _iter_lexicon_lines(path: Path):
    """Build helper: _iter_lexicon_lines."""
    with path.open("rb") as raw:
        head = raw.read(262144)
        raw.seek(0)
        if head.startswith(codecs.BOM_UTF8):
            raw.seek(len(codecs.BOM_UTF8))
            text = io.TextIOWrapper(raw, encoding="utf-8", errors="replace", newline=None)
        elif head.startswith(codecs.BOM_UTF16_LE):
            raw.seek(len(codecs.BOM_UTF16_LE))
            text = io.TextIOWrapper(raw, encoding="utf-16-le", errors="replace", newline=None)
        elif head.startswith(codecs.BOM_UTF16_BE):
            raw.seek(len(codecs.BOM_UTF16_BE))
            text = io.TextIOWrapper(raw, encoding="utf-16-be", errors="replace", newline=None)
        else:
            if _looks_utf16(head, le=True):
                text = io.TextIOWrapper(raw, encoding="utf-16-le", errors="replace", newline=None)
            else:
                chosen = "utf-8"
                for enc in ("utf-8", "gb18030"):
                    try:
                        head.decode(enc)
                        chosen = enc
                        break
                    except UnicodeDecodeError:
                        continue
                raw.seek(0)
                text = io.TextIOWrapper(raw, encoding=chosen, errors="replace", newline=None)
        yield from text


def validate_common_english_lexicon(lexicon_dir: Path) -> int:
    """Validate the checked-in 20,000-word native English lexicon."""
    path = lexicon_dir / COMMON_ENGLISH_LEXICON_RELATIVE_PATH
    license_path = lexicon_dir / COMMON_ENGLISH_LICENSE_RELATIVE_PATH
    readme_path = lexicon_dir / "en" / "README.md"
    for required_path in (path, license_path, readme_path):
        if not required_path.is_file() or required_path.stat().st_size <= 0:
            raise FileNotFoundError(f"missing English lexicon file: {required_path}")

    try:
        text = path.read_text(encoding="utf-8-sig")
    except UnicodeDecodeError as exc:
        raise RuntimeError(f"English lexicon must be UTF-8: {path}") from exc

    words: set[str] = set()
    previous_weight: int | None = None
    for line_no, raw in enumerate(text.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        fields = [field.strip() for field in line.split("\t")]
        if len(fields) != 3:
            raise RuntimeError(
                f"invalid English lexicon row {path}:{line_no}: expected exactly 3 tab-separated fields"
            )
        word, code, weight_text = fields
        if not (2 <= len(word) <= 24 and word.isascii() and word.isalpha() and word.islower()):
            raise RuntimeError(
                f"invalid English lexicon word {path}:{line_no}: {word!r}"
            )
        if code != word:
            raise RuntimeError(
                f"English input code must equal the word {path}:{line_no}: {code!r}"
            )
        try:
            weight = int(weight_text)
        except ValueError as exc:
            raise RuntimeError(
                f"invalid English lexicon weight {path}:{line_no}: {weight_text!r}"
            ) from exc
        if weight <= 0:
            raise RuntimeError(f"English lexicon weight must be positive {path}:{line_no}")
        if previous_weight is not None and weight > previous_weight:
            raise RuntimeError(
                f"English lexicon weights must be non-increasing {path}:{line_no}"
            )
        if word in words:
            raise RuntimeError(f"duplicate English lexicon word {path}:{line_no}: {word}")
        words.add(word)
        previous_weight = weight

    if len(words) != EXPECTED_COMMON_ENGLISH_WORDS:
        raise RuntimeError(
            f"English lexicon must contain exactly {EXPECTED_COMMON_ENGLISH_WORDS} words, "
            f"found {len(words)}: {path}"
        )
    missing = sorted(REQUIRED_COMMON_ENGLISH_WORDS - words)
    if missing:
        raise RuntimeError(f"English lexicon is missing required coverage words: {', '.join(missing)}")
    return len(words)


def ensure_corpus(repo: Path, force: bool = False) -> Path:
    """Ensure pinyin-ime/data/corpus.txt exists, generating it from lexicons if needed."""
    corpus = repo / "pinyin-ime" / "data" / "corpus.txt"
    _, source_fp = _corpus_sources_fingerprint(repo)
    fp_path = _corpus_fingerprint_path(corpus)
    if corpus.is_file() and not force:
        cached_fp = _read_sig(fp_path)
        if cached_fp == source_fp:
            print_msg(f"  {green('OK')} corpus exists: {corpus.name} ({fmt_size(corpus.stat().st_size)})")
            return corpus
        reason = "missing source fingerprint" if not cached_fp else "lexicon sources changed"
        print_msg(f"  {yellow('!')} corpus stale ({reason}); rebuilding")

    lex_dir = find_lexicon_dir(repo)
    source_files = _corpus_lexicon_source_files(repo)
    if not source_files:
        raise FileNotFoundError(f"No lexicon text files found under: {lex_dir}")
    frequency_calibrators = _build_frequency_calibrators(source_files)

    terms: dict[str, int] = {}
    zero_freq_lines = 0
    reweighted_from_flat = 0
    source_stats: dict[str, dict[str, int]] = {}
    for path in source_files:
        source_key = path.name
        source_stats[source_key] = {
            "lines": 0,
            "accepted": 0,
            "zero_freq": 0,
            "flat_reweighted": 0,
        }
        for line in _iter_lexicon_lines(path):
            source_stats[source_key]["lines"] += 1
            line = line.strip().lstrip("\ufeff")
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            term = parts[0].strip()
            if not term or not _has_cjk(term):
                continue
            w = _parse_lexicon_weight(parts)
            if w <= 0:
                zero_freq_lines += 1
                source_stats[source_key]["zero_freq"] += 1
            adjusted_w = _calibrated_source_weight(
                term,
                _adjust_weight_for_source(term, w, source_key),
                frequency_calibrators.get(path),
            )
            if w <= 1 and adjusted_w > 1:
                reweighted_from_flat += 1
                source_stats[source_key]["flat_reweighted"] += 1
            prev = terms.get(term)
            if prev is None or adjusted_w > prev:
                terms[term] = adjusted_w
            source_stats[source_key]["accepted"] += 1

    if not terms:
        raise RuntimeError(f"Could not generate valid corpus data from lexicons: {lex_dir}")

    corpus.parent.mkdir(parents=True, exist_ok=True)
    tmp = corpus.with_suffix(".txt.tmp")
    count = 0
    total_line_chars = 0
    with tmp.open("w", encoding="utf-8", newline="\n") as f:
        for term, w in sorted(terms.items(), key=lambda x: (-x[1], x[0])):
            n = _weight_repeats(w)
            for _ in range(n):
                f.write(term + "\n")
                count += 1
                total_line_chars += len(term)
    os.replace(tmp, corpus)
    _write_sig(fp_path, source_fp)

    _validate_corpus_from_stats(len(terms), count, total_line_chars)

    print_msg(f"  {green('OK')} generated corpus: {len(terms)} terms, {count} lines, {fmt_size(corpus.stat().st_size)}")
    if zero_freq_lines > 0:
        print_msg(
            f"  {yellow('!')} found and corrected freq<=0 rows: {zero_freq_lines} "
            f"(remapped to >=1 by source policy)"
        )
    if reweighted_from_flat > 0:
        print_msg(
            f"  {green('OK')} reweighted low-frequency source rows: {reweighted_from_flat}"
        )
    # Print only sources whose flat rows were reweighted to keep logs concise.
    for src, st in source_stats.items():
        if st["zero_freq"] == 0 and st["flat_reweighted"] == 0:
            continue
        print_msg(
            f"  {dim('-')} {src}: accepted {st['accepted']} rows, "
            f"zero/negative freq {st['zero_freq']}, reweighted {st['flat_reweighted']}"
        )
    return corpus


def _validate_corpus_from_stats(term_count: int, line_count: int, total_line_chars: int) -> None:
    """Build helper: _validate_corpus_from_stats."""
    warnings: list[str] = []
    if term_count < 100:
        warnings.append(f"too few terms ({term_count}); lexicon may be incomplete")
    if line_count < 500:
        warnings.append(f"too few corpus lines ({line_count}); check lexicon completeness")

    avg_len = total_line_chars / max(line_count, 1)
    if avg_len > 50:
        warnings.append(f"average line length is too long ({avg_len:.1f}); input format may be invalid")
    if avg_len < 1.5:
        warnings.append(f"average line length is too short ({avg_len:.1f}); lexicon parsing may have failed")

    for w in warnings:
        print_msg(f"  {yellow('!')} corpus quality: {w}")


# Path resolution

_HEX_COLOR_RE = re.compile(r"^#?[0-9a-fA-F]{6}$")


def _is_color_like(value: object) -> bool:
    return isinstance(value, str) and _HEX_COLOR_RE.match(value.strip()) is not None


def _parse_theme_color(value: object) -> tuple[int, int, int] | None:
    if not _is_color_like(value):
        return None
    text = str(value).strip().lstrip("#")
    return (int(text[0:2], 16), int(text[2:4], 16), int(text[4:6], 16))


def _relative_luminance(rgb: tuple[int, int, int]) -> float:
    def channel(v: int) -> float:
        s = v / 255.0
        return s / 12.92 if s <= 0.04045 else ((s + 0.055) / 1.055) ** 2.4

    r, g, b = rgb
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)


def _contrast_ratio(a: tuple[int, int, int], b: tuple[int, int, int]) -> float:
    la = _relative_luminance(a)
    lb = _relative_luminance(b)
    hi, lo = (la, lb) if la >= lb else (lb, la)
    return (hi + 0.05) / (lo + 0.05)


def _validate_theme_json(path: Path) -> list[str]:
    """Build helper: _validate_theme_json."""
    theme_name = f"{path.parent.name}/{path.name}"
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        raise RuntimeError(f"Could not read theme file: {path} ({e})") from e
    except OSError as e:
        raise RuntimeError(f"Could not read theme file: {path} ({e})") from e

    if not isinstance(data, dict):
        raise RuntimeError(f"Theme root must be a JSON object: {path}")

    warnings: list[str] = []
    required = ("name", "window_bg", "header_bg", "text", "selected_bg", "selected_text")
    missing = [k for k in required if k not in data]
    if missing:
        warnings.append(f"{theme_name}: missing required keys: {', '.join(missing)}")

    color_keys = [
        "window_bg", "window_bg_to", "header_bg", "header_bg_to",
        "border", "divider", "item_bg", "item_border", "hover_bg", "hover_border",
        "selected_bg", "selected_border", "pressed_bg", "pressed_border",
        "text", "muted_text", "selected_text", "selected_muted_text",
        "badge_bg", "badge_border", "badge_text",
        "chip_bg", "chip_border", "chip_text",
        "chip_active_bg", "chip_active_border", "chip_active_text",
        "selected_outline",
    ]
    for key in color_keys:
        if key in data and data[key] is not None and not _is_color_like(data[key]):
            warnings.append(f"{theme_name}: field {key} is not a valid color (#RRGGBB expected)")

    def color_for(key: str, fallback: str | None = None) -> tuple[int, int, int] | None:
        return _parse_theme_color(data.get(key)) or (
            _parse_theme_color(data.get(fallback)) if fallback else None
        )

    contrast_checks = [
        ("text", "item_bg", "window_bg", 4.2),
        ("muted_text", "item_bg", "window_bg", 3.2),
        ("selected_text", "selected_bg", None, 4.2),
        ("selected_muted_text", "selected_bg", None, 3.2),
        ("badge_text", "badge_bg", None, 3.6),
        ("chip_text", "chip_bg", None, 3.6),
        ("chip_active_text", "chip_active_bg", None, 4.2),
    ]
    for fg_key, bg_key, bg_fallback, min_ratio in contrast_checks:
        fg = color_for(fg_key)
        bg = color_for(bg_key, bg_fallback)
        if not fg or not bg:
            continue
        ratio = _contrast_ratio(fg, bg)
        if ratio < min_ratio:
            warnings.append(
                f"{theme_name}: {fg_key} on {bg_key} contrast is {ratio:.2f}:1 "
                f"(recommended >= {min_ratio:.1f}:1)"
            )

    for key in ("shadow_enabled", "animations_enabled"):
        if key in data and not isinstance(data[key], bool):
            warnings.append(f"{theme_name}: {key} should be boolean")
    numeric_ranges = {
        "selected_accent_width": (0, 8),
        "selected_ring_opacity": (0.0, 1.0),
        "border_opacity": (0.0, 1.0),
        "divider_opacity": (0.0, 1.0),
        "shadow_opacity": (0.0, 1.0),
        "shadow_size": (0, 24),
        "font_weight": (300, 700),
        "selected_font_weight": (400, 800),
        "label_font_weight": (400, 800),
        "chip_font_weight": (350, 700),
        "show_animation_ms": (0, 240),
        "selection_animation_ms": (0, 240),
        "hover_animation_ms": (0, 240),
        "press_animation_ms": (0, 240),
        "page_animation_ms": (0, 240),
    }
    for key, (lo, hi) in numeric_ranges.items():
        if key not in data:
            continue
        value = data[key]
        if not isinstance(value, (int, float)) or isinstance(value, bool) or value < lo or value > hi:
            warnings.append(f"{theme_name}: {key} should be numeric in range {lo}..{hi}")
    if "selected_indicator" in data and data["selected_indicator"] not in (
        "left_bar", "bottom_bar", "outline", "none"
    ):
        warnings.append(
            f"{theme_name}: selected_indicator should be left_bar, bottom_bar, outline, or none"
        )

    return warnings


def step_validate_skins(repo: Path) -> list[str]:
    """Build helper: step_validate_skins."""
    skins_root = repo / "skins"
    if not skins_root.is_dir():
        raise FileNotFoundError(f"skins directory not found: {skins_root}")
    theme_files = sorted(skins_root.glob("*/theme.json"), key=lambda p: p.as_posix().lower())
    if not theme_files:
        raise FileNotFoundError(f"no skin theme.json found: {skins_root}")

    warnings: list[str] = []
    for theme in theme_files:
        warnings.extend(_validate_theme_json(theme))
    print_msg(f"  {green('OK')} skin validation passed: {len(theme_files)} themes")
    return warnings


# UTF-8 source check

def step_check_utf8_sources(repo: Path) -> None:
    """Build helper: step_check_utf8_sources."""
    script = repo / "scripts" / "check_utf8_sources.py"
    if not script.is_file():
        raise FileNotFoundError(f"UTF-8 check script not found: {script}")
    if ctx.dry_run:
        print_msg(f"  [dry-run] python \"{script}\" --root \"{repo}\"")
        return
    run([sys.executable, str(script), "--root", str(repo)], cwd=repo)


def step_check_lexicon_syllables(repo: Path) -> None:
    """Check that Chinese lexicon pinyin codes are accepted by syllables.txt."""
    script = repo / "scripts" / "check_lexicon_syllables.py"
    if not script.is_file():
        raise FileNotFoundError(f"lexicon syllable check script not found: {script}")
    if ctx.dry_run:
        print_msg(f"  [dry-run] python \"{script}\" --root \"{repo}\"")
        return
    run(
        [
            sys.executable,
            str(script),
            "--root",
            str(repo),
            "--check-syllable-count",
            "--strict-syllable-count",
        ],
        cwd=repo,
    )


def step_verify_rust(repo: Path) -> None:
    """Run the correctness gates that a successful packaging build must satisfy."""
    cargo = "cargo" if ctx.dry_run else which_or_die(
        "cargo", "Install Rust from https://rustup.rs"
    )
    cargo_dir = repo / "pinyin-ime"
    cargo_env = {"CARGO_REGISTRIES_CRATES_IO_PROTOCOL": "sparse"}
    run([cargo, "fmt", "--check"], cwd=cargo_dir, env=cargo_env)
    run(
        [cargo, "check", "--locked", "--all-targets"],
        cwd=cargo_dir,
        env=cargo_env,
    )


# CMake generator cache

def step_regenerate_lexicons(repo: Path) -> None:
    """Regenerate native lexicons and the supported-character table."""
    script = repo / "scripts" / "regenerate_lexicons.ps1"
    if not script.is_file():
        raise FileNotFoundError(f"Missing lexicon regeneration script: {script}")
    chars = repo / "pinyin-ime" / "data" / "pinyin_supported_chars.txt"
    if ctx.dry_run:
        print_msg(f"  [dry-run] powershell -File \"{script}\"")
        return
    powershell = which_or_die("powershell", "PowerShell is required to regenerate lexicons")
    run(
        [powershell, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script)],
        cwd=repo,
    )
    try:
        st = chars.stat()
        print_msg(f"  {green('OK')} lexicons and character table updated: {chars.name} ({fmt_size(st.st_size)})")
    except OSError:
        pass


# Incremental build checks

def stop_workspace_rust_artifact_processes(
    repo: Path,
    artifact_names: list[str] | tuple[str, ...],
) -> None:
    """Release running binaries built inside this workspace before Cargo replaces them.

    Only processes whose executable path is under ``pinyin-ime/target`` are
    stopped. Installed copies elsewhere on the machine are deliberately left
    alone.
    """
    _stop_processes_under_root(
        repo, artifact_names, (repo / "pinyin-ime" / "target").resolve()
    )


def _stop_processes_under_root(
    repo: Path,
    artifact_names: list[str] | tuple[str, ...],
    target_root: Path,
) -> None:
    if ctx.dry_run or os.name != "nt":
        return
    process_names = sorted(
        {Path(name).name for name in artifact_names if str(name).lower().endswith(".exe")}
    )
    if not process_names:
        return
    powershell = shutil.which("powershell.exe") or shutil.which("powershell")
    if not powershell:
        return
    script = r"""
$targetRoot = [IO.Path]::GetFullPath($env:KAIXIN_BUILD_TARGET_ROOT).TrimEnd('\') + '\'
$names = $env:KAIXIN_BUILD_PROCESS_NAMES -split ';'
$processes = @()
try {
    $processes = @(Get-CimInstance Win32_Process -ErrorAction Stop | ForEach-Object {
        if ($_.ExecutablePath) {
            [PSCustomObject]@{ Name = $_.Name; Id = $_.ProcessId; Path = $_.ExecutablePath }
        }
    })
} catch {
    # Standard-user and sandboxed sessions may be denied Win32_Process access.
    # Get-Process still exposes Path for the current user's processes, which is
    # sufficient because only executables under this workspace may be stopped.
    Write-Output "WARNING Win32_Process enumeration unavailable; using Get-Process fallback"
    $baseNames = @($names | ForEach-Object { [IO.Path]::GetFileNameWithoutExtension($_) } | Select-Object -Unique)
    $processes = @(Get-Process -Name $baseNames -ErrorAction SilentlyContinue | ForEach-Object {
        $processPath = $null
        try { $processPath = $_.Path } catch { $processPath = $null }
        if ($processPath) {
            [PSCustomObject]@{ Name = ($_.ProcessName + '.exe'); Id = $_.Id; Path = $processPath }
        }
    })
}
$processes | Where-Object {
    $_.Path -and
    $names -contains $_.Name -and
    ([IO.Path]::GetFullPath($_.Path).StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase))
} | ForEach-Object {
    Write-Output ("STOPPED {0} pid={1}" -f $_.Name, $_.Id)
    Stop-Process -Id $_.Id -Force -ErrorAction Stop
}
"""
    env = os.environ.copy()
    env["KAIXIN_BUILD_TARGET_ROOT"] = str(target_root)
    env["KAIXIN_BUILD_PROCESS_NAMES"] = ";".join(process_names)
    result = subprocess.run(
        [powershell, "-NoProfile", "-NonInteractive", "-Command", script],
        cwd=str(repo),
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    output = result.stdout.strip()
    if result.returncode != 0:
        raise RuntimeError(
            "failed to stop workspace Rust process before build"
            + (f":\n{output}" if output else "")
        )
    for line in output.splitlines():
        if line.startswith("STOPPED "):
            print_msg(f"  {yellow('!')} {line}; releasing Cargo output lock")
        elif line.startswith("WARNING "):
            print_msg(f"  {yellow('!')} {line}")

def step_cargo(
    repo: Path,
    profile: str,
    arch: str = ARCH_X64,
    artifact_names: list[str] | tuple[str, ...] | None = None,
) -> None:
    """Build helper: step_cargo."""
    cargo = "cargo" if ctx.dry_run else which_or_die("cargo", "Install Rust from https://rustup.rs")
    cargo_dir = repo / "pinyin-ime"
    cmd = [cargo, "build", "--locked"]
    if profile == "release":
        cmd.append("--release")
    target = RUST_TARGET_BY_ARCH.get(arch)
    if target:
        cmd += ["--target", target]
    artifact_names = artifact_names or rust_artifact_names_for_variants()
    stop_workspace_rust_artifact_processes(repo, artifact_names)
    bin_names = rust_bin_names_for_artifacts(artifact_names)
    if bin_names:
        for bin_name in bin_names:
            cmd += ["--bin", bin_name]
    elif target:
        cmd.append("--lib")
    cargo_env = {
        # Cargo's legacy git index depends on GitHub, which is often much less
        # reachable than the official sparse index on Windows build machines.
        "CARGO_REGISTRIES_CRATES_IO_PROTOCOL": "sparse",
    }
    if not ctx.quiet:
        print_msg(
            f"  {dim('>> Cargo may stay on `Updating crates.io index` while resolving or downloading')}"
        )
        print_msg(
            f"  {dim('>> first build downloads locked dependencies into %USERPROFILE%\\.cargo\\registry')}"
        )
    run(cmd, cwd=cargo_dir, env=cargo_env)
    _refresh_cargo_build_sig(repo, profile, arch)


def step_input_perf_smoke(
    repo: Path,
    profile: str,
    max_p99_us: int,
    max_incremental_p99_us: int,
) -> None:
    """Run a deterministic local release performance smoke test with an isolated profile."""
    if profile != "release":
        raise RuntimeError("--perf-smoke requires a Release build (remove --debug)")
    if max_p99_us <= 0:
        raise RuntimeError("--perf-max-p99-us must be greater than zero")
    if max_incremental_p99_us <= 0:
        raise RuntimeError("--perf-max-incremental-p99-us must be greater than zero")
    perf_tool = rust_target_dir(repo, profile, ARCH_X64) / PERF_RUST_ARTIFACT
    if not ctx.dry_run and not perf_tool.is_file():
        raise FileNotFoundError(f"missing performance smoke tool: {perf_tool}")
    perf_runs = [
        [
            str(perf_tool),
            "--workload-size",
            "500",
            "--warmup",
            "2",
            "--iterations",
            "2",
            "--max-p99-us",
            str(max_p99_us),
        ],
        [
            str(perf_tool),
            "--incremental",
            "--workload-size",
            "500",
            "--warmup",
            "0",
            "--iterations",
            "1",
            "--max-p99-us",
            str(max_incremental_p99_us),
        ],
    ]
    if ctx.dry_run:
        for command in perf_runs:
            run(command, cwd=repo / "pinyin-ime")
        return
    with tempfile.TemporaryDirectory(prefix="kaixin-input-perf-") as tmp:
        smoke_root = Path(tmp)
        local_appdata = smoke_root / "LocalAppData"
        local_appdata.mkdir(parents=True, exist_ok=True)
        smoke_env = {
            "LOCALAPPDATA": str(local_appdata),
            "SRF_USER_DICT_PATH": str(smoke_root / "user_dict.sqlite"),
            "SRF_IME_LOG_LEVEL": "error",
        }
        for command in perf_runs:
            run(command, cwd=repo / "pinyin-ime", env=smoke_env)


def step_cmake(repo: Path, tsf_tip: Path, build_dir: Path, profile: str, clean: bool,
               arch: str = ARCH_X64) -> None:
    """Build helper: step_cmake."""
    if ctx.dry_run:
        print_msg(f"  [dry-run] cmake configure/build ({profile}, {arch}) at {build_dir}")
        return
    cmake = which_or_die("cmake", "Install CMake: https://cmake.org/download/")

    if clean and build_dir.exists():
        print_msg(f"  clean: {build_dir}")
        shutil.rmtree(build_dir, ignore_errors=True)

    build_dir.mkdir(parents=True, exist_ok=True)

    # ---------------------------------------------------------------------------
    cache = build_dir / "CMakeCache.txt"
    if cache.is_file():
        try:
            text = cache.read_text(encoding="utf-8", errors="ignore")
            for line in text.splitlines():
                if line.startswith("CMAKE_HOME_DIRECTORY:"):
                    cached = line.partition("=")[2].strip()
                    if os.path.normcase(os.path.normpath(cached)) != os.path.normcase(os.path.normpath(str(tsf_tip))):
                        print_msg(f"  {yellow('!')} CMake cache source directory mismatch; cleaning")
                        shutil.rmtree(build_dir, ignore_errors=True)
                        build_dir.mkdir(parents=True, exist_ok=True)
                    break
        except OSError:
            pass

    # ---------------------------------------------------------------------------
    gen_cache = build_dir / ".cmake_generator_cache"
    cached_gen = load_cached_generator(gen_cache, arch) if gen_cache else None

    generators = detect_vs_generators(cmake, arch)
    if cached_gen:
        # Try the last successful generator first.
        for i, (name, _) in enumerate(generators):
            if name == cached_gen[0]:
                generators.insert(0, generators.pop(i))
                break
        else:
            generators.insert(0, cached_gen)

    ninja = shutil.which("ninja")

    last_err = None
    for gen_name, gen_args in generators:
        try:
            run([cmake, "-S", str(tsf_tip), "-B", str(build_dir), "-G", gen_name, *gen_args], cwd=repo)
        except BuildCommandError as e:
            last_err = e
            shutil.rmtree(build_dir, ignore_errors=True)
            build_dir.mkdir(parents=True, exist_ok=True)
            continue

        build_cmd = [cmake, "--build", str(build_dir), "--parallel"]
        if "Ninja" not in gen_name:
            build_cmd += ["--config", profile.capitalize()]
        build_env = {"MSBUILDDISABLENODEREUSE": "1"}
        try:
            run(build_cmd, cwd=repo, env=build_env)
        except BuildCommandError as exc:
            transient_object_lock = any(
                marker in line.lower()
                for line in exc.tail_lines
                for marker in (
                    "permission denied",
                    "access is denied",
                    "lnk1104",
                    "无法打开编译器生成的文件",
                    "无法打开文件",
                    "拒绝访问",
                )
            )
            if transient_object_lock:
                if not ctx.quiet:
                    print_msg(
                        yellow(
                            "! MSBuild hit a transient object-file lock; retrying this "
                            "architecture once with one build worker"
                        )
                    )
                time.sleep(0.5)
                retry_cmd = [cmake, "--build", str(build_dir), "--parallel", "1"]
                if "Ninja" not in gen_name:
                    retry_cmd += ["--config", profile.capitalize()]
                run(retry_cmd, cwd=repo, env=build_env)
            else:
                if not ctx.quiet:
                    print_msg(
                        red(
                            f"! CMake build failed after {gen_name} configured successfully; "
                            "not retrying other generators"
                        )
                    )
                raise
        except Exception:
            if not ctx.quiet:
                print_msg(
                    red(
                        f"! CMake build failed after {gen_name} configured successfully; "
                        "not retrying other generators"
                    )
                )
            raise
        if gen_cache:
            save_cached_generator(gen_cache, gen_name, gen_args)
        _refresh_cmake_build_sig(tsf_tip, build_dir, profile)
        return

    # Fall back to Ninja if no Visual Studio generator succeeds.
    if ninja:
        try:
            cmake_type = "Release" if profile == "release" else "Debug"
            run([cmake, "-S", str(tsf_tip), "-B", str(build_dir), "-G", "Ninja",
                 f"-DCMAKE_BUILD_TYPE={cmake_type}"], cwd=repo)
            run([cmake, "--build", str(build_dir), "--parallel"], cwd=repo)
            if gen_cache:
                save_cached_generator(gen_cache, "Ninja", [])
            _refresh_cmake_build_sig(tsf_tip, build_dir, profile)
            return
        except BuildCommandError as e:
            last_err = e

    raise RuntimeError(f"CMake build failed. Last exit code: {last_err.returncode if last_err else '?'}")


def step_stage(repo: Path, tsf_tip: Path, tip_dll: Path, overlay_exe: Path,
               x86_tip_dll: Path, x86_overlay_exe: Path,
               output_root: Path, powershell: Path,
               make_zip: bool, profile_cap: str,
               include_ocr: bool = True,
               include_translation: bool = False) -> None:
    """Build helper: step_stage."""
    script = tsf_tip / "installer" / "stage_package.ps1"
    if not script.is_file():
        raise FileNotFoundError(f"Missing character table expansion script: {script}")

    cmd = [
        str(powershell), "-NoProfile", "-ExecutionPolicy", "Bypass",
        "-File", str(script),
        "-OutputRoot", str(output_root),
        "-TipDllPath", str(tip_dll),
        "-OverlayExePath", str(overlay_exe),
        "-X86TipDllPath", str(x86_tip_dll),
        "-X86OverlayExePath", str(x86_overlay_exe),
        "-Profile", profile_cap,
        "-IncludeOcr", "1" if include_ocr else "0",
    ]
    if make_zip:
        cmd.append("-Zip")
    run(cmd, cwd=repo)


def step_inno(repo: Path, script_name: str = "kaixin.iss", output_name: str = "kaixin-setup.exe",
              package_dir: Path | None = None, include_ocr: bool = True,
              include_translation: bool = False) -> Path:
    """Run Inno Setup and return the generated installer path."""
    iss = repo / "tsf-tip" / "installer" / script_name
    define_args: list[str] = []
    if not ctx.no_version:
        define_args.append(f"/DKXAppVersion={ctx.version}")
    if package_dir is not None:
        define_args.append(f"/DKXPackageDir={package_dir}")
    define_args.append(f"/DKXOutputBaseFilename={Path(output_name).stem}")
    define_args.append(f"/DKXIncludeOcr={'1' if include_ocr else '0'}")
    define_args.append(f"/DKXIncludeTranslation={'1' if include_translation else '0'}")
    if package_dir is not None and not ctx.dry_run:
        component_manifest = package_dir / "component_manifest.ini"
        if not component_manifest.is_file():
            raise FileNotFoundError(f"missing installer component manifest: {component_manifest}")
        component_config = configparser.ConfigParser(interpolation=None)
        component_config.read(component_manifest, encoding="utf-8-sig")
        component_defines = {
            "python_runtime": "KXComponentPythonRuntime",
            "rapidocr_packages": "KXComponentRapidOcrPackages",
            "rapidocr_payload": "KXComponentRapidOcrPayload",
        }
        for key, define_name in component_defines.items():
            component_id = component_config.get("components", key, fallback="").strip().lower()
            if component_id != "absent" and not re.fullmatch(r"[0-9a-f]{64}", component_id):
                raise RuntimeError(
                    f"invalid component id {key!r} in {component_manifest}: {component_id!r}"
                )
            define_args.append(f"/D{define_name}={component_id}")
    if ctx.dry_run:
        print_msg(f"  [dry-run] ISCC {' '.join(define_args)} {script_name}")
        return repo / "dist" / output_name
    iscc = find_iscc()
    if iscc is None:
        raise FileNotFoundError(
            "Inno Setup (ISCC.exe) not found.\n"
            "  Install Inno Setup 6: https://jrsoftware.org/isinfo.php\n"
            "  Or use --no-inno to skip this step."
        )
    if not iss.is_file():
        raise FileNotFoundError(f"Inno script not found: {iss}")

    cmd = [str(iscc)]
    cmd.extend(define_args)
    cmd.append(str(iss))
    run(cmd, cwd=repo)

    output = repo / "dist" / output_name
    if not output.is_file():
        raise RuntimeError(f"Inno Setup finished but the installer was not found: {output}")
    return output


# Corpus generation

def step_cargo_and_cmake_parallel(
    repo: Path,
    tsf_tip: Path,
    cmake_build_dir: Path,
    profile: str,
    clean: bool,
    *,
    cmake_build_dir_x86: Path | None = None,
    build_x86: bool = False,
    rust_artifact_names: list[str] | tuple[str, ...] | None = None,
) -> None:
    """Build helper: step_cargo_and_cmake_parallel."""
    tasks: list[tuple[str, Callable[[], None]]] = [
        ("Rust", lambda: step_cargo(repo, profile, artifact_names=rust_artifact_names)),
        ("C++ x64", lambda: step_cmake(repo, tsf_tip, cmake_build_dir, profile, clean=clean)),
    ]
    if build_x86 and cmake_build_dir_x86 is not None:
        tasks.append(
            (
                "C++ x86",
                lambda: step_cmake(
                    repo,
                    tsf_tip,
                    cmake_build_dir_x86,
                    profile,
                    clean=clean,
                    arch=ARCH_X86,
                ),
            )
        )

    errors = run_parallel_tasks(tasks, context="build")
    if errors:
        raise errors[0][1]


def smoke_verify(repo: Path, profile: str) -> list[str]:
    """Pre-stage smoke checks for build outputs that already exist."""
    warnings: list[str] = []
    profile_cap = profile.capitalize()
    tsf_tip = repo / "tsf-tip"

    for arch in BUILD_ARCHES:
        try:
            tsf_dll = find_tsf_dll(tsf_tip, cmake_build_dir_for(tsf_tip, arch), profile_cap)
            try:
                machine = read_pe_machine(tsf_dll)
                expected = PE_MACHINE_BY_ARCH[arch]
                if machine != expected:
                    warnings.append(
                        f"srf_tsf_tip.dll {arch} mismatch (Machine=0x{machine:04X}, expected 0x{expected:04X})"
                    )
            except Exception:
                warnings.append(f"unable to read srf_tsf_tip.dll PE header for {arch}")
        except FileNotFoundError:
            warnings.append(f"srf_tsf_tip.dll {arch} not found; skipping architecture check")

    return warnings


def post_stage_smoke_verify(repo: Path, output_root: Path, profile: str) -> list[str]:
    """Smoke-check the staged package produced in this build."""
    warnings: list[str] = []
    for name in ("lexicon.bin", "hot_lexicon.bin"):
        lex_bin = output_root / "lexicon" / name
        try:
            schema = verify_prebaked_lexicon(lex_bin)
            print_msg(
                f"  {green('OK')} {name}: SRFLX002 schema v{schema} "
                f"({fmt_size(lex_bin.stat().st_size)})"
            )
        except (FileNotFoundError, RuntimeError) as exc:
            warnings.append(str(exc))

    return warnings


def _stamped_git_commit_in(data: bytes, commit_short: str) -> bool:
    """True when a staged binary embeds the expected 12-hex commit, in ASCII
    (narrow C++/Rust string literals) or UTF-16LE (defensive)."""
    needle = commit_short.encode("ascii")
    return needle in data or needle.decode("ascii").encode("utf-16-le") in data


def _best_effort_git_commit(data: bytes) -> str | None:
    """Best-effort extraction of an exactly-12-hex run for error messages only.
    Never used for the pass/fail decision: the real DLL contains 12-hex runs
    that are not the commit, while membership of the expected commit is
    unambiguous."""
    m = re.search(rb"(?<![0-9a-f])[0-9a-f]{12}(?![0-9a-f])", data)
    return m.group().decode("ascii") if m else None


def verify_staged_git_commits(runtime_dir: Path, engine_exe: Path, repo: Path) -> None:
    """Build helper: verify_staged_git_commits.

    Hard packaging gate: staged x64/x86 TIP DLLs and srf_ime_engine.exe must
    all embed the git commit HEAD has at staging time.  Mixed or stale stamps
    fail the package.  Degrades to binary-vs-binary consistency when git is
    unavailable.  The dirty flag is not part of the decision (DLLs embed it
    only as an integer constant, not a string)."""
    info = git_release_info(repo)
    expected = info.get("commit_short")
    targets = [
        ("x64 srf_tsf_tip.dll", runtime_dir / "x64" / "srf_tsf_tip.dll"),
        ("x86 srf_tsf_tip.dll", runtime_dir / "x86" / "srf_tsf_tip.dll"),
        ("srf_ime_engine.exe", engine_exe),
    ]
    found: dict[str, str | None] = {}
    for name, path in targets:
        data = path.read_bytes()
        if expected and _stamped_git_commit_in(data, expected):
            found[name] = expected
        else:
            # Display-only: prefer the real stamped hex over the "unknown"
            # fallback word, which also appears in binaries for unrelated
            # reasons (error strings).  The decision stays membership-based.
            found[name] = _best_effort_git_commit(data) or (
                "unknown" if b"unknown" in data else None
            )
    stamps = list(found.values())
    if expected:
        ok = all(s == expected for s in stamps)
    else:
        ok = all(s is not None and s == stamps[0] for s in stamps)
    if ok:
        print_msg(
            f"  {green('OK')} git stamps match HEAD ({expected or 'unknown'}) "
            "in all staged binaries"
        )
        return
    detail = ", ".join(f"{name}={stamp or '?'}" for name, stamp in found.items())
    head_note = f"当前 HEAD={expected}" if expected else "git 不可用，仅校验二进制间一致性"
    raise RuntimeError(
        f"staged binaries carry stale or mismatched git stamps: {detail}; "
        f"{head_note}. 各产物必须在同一份新代码上重新打包：请重新运行完整构建，"
        "不要使用 --quick / --skip-cargo / --skip-cmake，"
        "确保 DLL 与引擎使用同一 git 提交（只用新代码）。"
    )


def verify_staged_package(
    output_root: Path,
    include_ocr: bool = True,
    include_translation: bool = False,
    repo: Path | None = None,
) -> None:
    """Build helper: verify_staged_package."""
    if not output_root.is_dir():
        raise FileNotFoundError(f"stage directory does not exist: {output_root}")
    verify_package_hash_manifest(output_root)
    marker = output_root / "current_runtime_payload.txt"
    if not marker.is_file():
        raise FileNotFoundError(f"missing current_runtime_payload.txt: {marker}")
    rel = marker.read_text(encoding="utf-8", errors="replace").strip()
    if not rel:
        raise RuntimeError(f"{marker.name} is empty; cannot locate runtime directory")
    runtime_dir = (output_root / rel).resolve()
    try:
        runtime_dir.relative_to(output_root.resolve())
    except ValueError as e:
        raise RuntimeError(f"invalid runtime path outside stage: {rel}") from e
    for arch in BUILD_ARCHES:
        arch_dir = runtime_dir / arch
        p = arch_dir / "srf_tsf_tip.dll"
        if not p.is_file():
            raise FileNotFoundError(f"missing staged {arch} srf_tsf_tip.dll: {p}")
        assert_pe_arch(p, arch, f"staged {arch} srf_tsf_tip.dll")
        overlay = arch_dir / "srf_ime_overlay.exe"
        if not overlay.is_file():
            raise FileNotFoundError(f"missing staged {arch} srf_ime_overlay.exe: {overlay}")
        assert_pe_arch(overlay, arch, f"staged {arch} srf_ime_overlay.exe")
    for name in BASE_STAGED_RUST_ARTIFACTS:
        p = output_root / name
        if not p.is_file():
            raise FileNotFoundError(f"missing staged {name}: {p}")
    ocr_exe = output_root / "srf_ime_ocr.exe"
    if include_ocr:
        if not ocr_exe.is_file():
            raise FileNotFoundError(f"missing staged srf_ime_ocr.exe: {ocr_exe}")
    elif ocr_exe.exists():
        raise RuntimeError(f"IME-only package must not include OCR extension: {ocr_exe}")

    rapidocr_root = output_root / RAPIDOCR_DIR
    rapidocr_python_package = rapidocr_root / "python" / "rapidocr"
    rapidocr_model_dir = rapidocr_python_package / "models"
    legacy_venv = output_root / RAPIDOCR_VENV
    if legacy_venv.exists():
        raise RuntimeError(f"staged package must not contain legacy OCR venv: {legacy_venv}")
    if include_ocr:
        rapidocr_required = [
            output_root / "tools" / RAPIDOCR_HELPER,
            output_root / "tools" / OPENCV_CROP_HELPER,
            rapidocr_python_package / "__init__.py",
            rapidocr_packages_path(output_root),
            packaged_python_runtime_path(output_root),
            packaged_python_stdlib_marker(output_root),
        ]
        rapidocr_required.extend(rapidocr_model_dir / model for model in RAPIDOCR_REQUIRED_MODELS)
        for path in rapidocr_required:
            if path.is_dir():
                continue
            if not path.is_file():
                raise FileNotFoundError(f"missing staged RapidOCR runtime file: {path}")
            if path.stat().st_size <= 0:
                raise RuntimeError(f"staged RapidOCR runtime file is empty: {path}")
        python_runtime_root = output_root / PYTHON_RUNTIME
        if sys.platform == "win32" and not any(python_runtime_root.glob("python*.dll")):
            raise FileNotFoundError(f"missing staged Python runtime DLLs: {python_runtime_root}")
    else:
        forbidden_ocr_paths = [
            rapidocr_root,
            output_root / RAPIDOCR_PACKAGES,
            output_root / PYTHON_RUNTIME,
            *(output_root / "tools" / helper for helper in RAPIDOCR_HELPERS),
        ]
        for path in forbidden_ocr_paths:
            if path.exists():
                raise RuntimeError(f"IME-only package must not include OCR runtime file: {path}")

    forbidden_translation_paths = [
        output_root / "srf_ime_translate.exe",
        output_root / ".venv-translate",
        output_root / "models" / "translate",
        output_root / "tools" / "kaixin_translate_engine.py",
        output_root / "tools" / "kaixin_translate_engine.cmd",
    ]
    for path in forbidden_translation_paths:
        if path.exists():
            raise RuntimeError(f"package must not include the removed translation runtime: {path}")

    python_roots = [
        output_root / PYTHON_RUNTIME,
        output_root / RAPIDOCR_PACKAGES,
        output_root / RAPIDOCR_DIR / "python",
    ]
    forbidden_python_artifacts: list[str] = []
    for python_root in python_roots:
        if not python_root.is_dir():
            continue
        for path in python_root.rglob("*"):
            relative = path.relative_to(output_root).as_posix()
            name = path.name.casefold()
            if path.is_dir() and (
                name in PYTHON_PACKAGE_EXCLUDED_DIR_NAMES
                or (name.startswith("pip-") and name.endswith(".dist-info"))
                or (name.startswith("setuptools-") and name.endswith(".dist-info"))
            ):
                forbidden_python_artifacts.append(relative)
            elif path.is_file() and path.suffix.casefold() in PYTHON_PACKAGE_EXCLUDED_FILE_SUFFIXES:
                forbidden_python_artifacts.append(relative)
            if len(forbidden_python_artifacts) >= 20:
                break
        if len(forbidden_python_artifacts) >= 20:
            break
    if forbidden_python_artifacts:
        raise RuntimeError(
            "staged Python runtimes contain excluded cache/test/development artifacts: "
            + ", ".join(forbidden_python_artifacts)
        )

    icon_file = output_root / "assets" / "kaixin-input.ico"
    if not icon_file.is_file():
        raise FileNotFoundError(
            f"missing staged Win+Space/profile icon file: {icon_file}"
        )

    staged_lexicon_dir = output_root / "lexicon"
    english_count = validate_common_english_lexicon(staged_lexicon_dir)
    legacy_english_files = [
        path
        for path in (staged_lexicon_dir / "en").iterdir()
        if path.is_file()
        and (
            path.suffix.casefold() in {".sqlite", ".aff", ".dic"}
            or "hunspell" in path.name.casefold()
            or "scowl" in path.name.casefold()
        )
    ]
    if legacy_english_files:
        raise RuntimeError(
            "staged package contains obsolete large English dictionary files: "
            + ", ".join(path.name for path in legacy_english_files)
        )
    print_msg(f"  {green('OK')} staged English lexicon: {english_count} words")

    for name in ("lexicon.bin", "hot_lexicon.bin"):
        lexicon_bin = staged_lexicon_dir / name
        schema = verify_prebaked_lexicon(lexicon_bin)
        print_msg(f"  {green('OK')} staged {name}: SRFLX002 schema v{schema}")

    settings_exe = output_root / "srf_ime_settings.exe"
    clipboard_exe = output_root / "srf_ime_clipboard.exe"
    handwrite_exe = output_root / "srf_ime_handwrite.exe"
    ocr_exe = output_root / "srf_ime_ocr.exe"
    if settings_exe.stat().st_size == clipboard_exe.stat().st_size:
        settings_hash = sha256_file(settings_exe)
        clipboard_hash = sha256_file(clipboard_exe)
        if settings_hash == clipboard_hash:
            raise RuntimeError(
                "staged srf_ime_settings.exe and srf_ime_clipboard.exe are identical; "
                "check Rust bin targets and staging copy rules"
            )
    settings_bytes = settings_exe.read_bytes()
    clipboard_bytes = clipboard_exe.read_bytes()
    handwrite_bytes = handwrite_exe.read_bytes()
    ocr_bytes = ocr_exe.read_bytes() if include_ocr else b""
    if "开心输入法 剪贴板".encode("utf-8") in settings_bytes:
        raise RuntimeError(
            "staged srf_ime_settings.exe contains clipboard window title; "
            "rebuild --bin srf_ime_settings and check staging copy rules"
        )
    if "开心输入法 设置".encode("utf-8") not in settings_bytes:
        raise RuntimeError("staged srf_ime_settings.exe is missing settings window title")
    if "开心输入法 剪贴板".encode("utf-8") not in clipboard_bytes:
        raise RuntimeError("staged srf_ime_clipboard.exe is missing clipboard window title")
    clipboard_svc_exe = output_root / "srf_ime_clipboard_svc.exe"
    clipboard_svc_bytes = clipboard_svc_exe.read_bytes()
    if "开心输入法 剪贴板".encode("utf-8") in clipboard_svc_bytes:
        raise RuntimeError(
            "staged srf_ime_clipboard_svc.exe contains clipboard window title; "
            "the Rust service must be headless — check srf_ime_clipboard_svc.rs"
        )
    if "开心输入法 手写查字".encode("utf-8") not in handwrite_bytes:
        raise RuntimeError("staged srf_ime_handwrite.exe is missing handwriting window title")
    if include_ocr and "开心输入法 OCR".encode("utf-8") not in ocr_bytes:
        raise RuntimeError("staged srf_ime_ocr.exe is missing OCR window title")

    if repo is not None:
        verify_staged_git_commits(runtime_dir, output_root / "srf_ime_engine.exe", repo)

    install_dev = output_root / "install_dev.ps1"
    install_current_user = output_root / "install_current_user.ps1"
    uninstall_dev = output_root / "uninstall_dev.ps1"
    uninstall_current_user = output_root / "uninstall_current_user.ps1"
    restart_stale_hosts = output_root / "restart_stale_hosts.ps1"
    repair_install = output_root / "repair_install.ps1"
    export_diagnostics = output_root / "export_diagnostics.ps1"
    invoke_registration = output_root / "invoke_registration.ps1"
    user_data_manifest = output_root / "user_data_manifest.json"
    version_file = output_root / "VERSION"
    third_party_notices = output_root / "THIRD_PARTY_NOTICES.md"
    project_license_files = [
        output_root / "LICENSE",
        output_root / "LICENSE_SCOPE.md",
        output_root / "NOTICE",
    ]
    for script in (
        install_dev,
        install_current_user,
        uninstall_dev,
        uninstall_current_user,
        restart_stale_hosts,
        repair_install,
        export_diagnostics,
        invoke_registration,
    ):
        if not script.is_file():
            raise FileNotFoundError(f"missing staged installer script: {script}")
        if not script.read_bytes().startswith(codecs.BOM_UTF8):
            raise RuntimeError(
                f"staged PowerShell script must use UTF-8 BOM for Windows PowerShell 5.1: {script}"
            )
    if not version_file.is_file():
        raise FileNotFoundError(f"missing staged VERSION file: {version_file}")
    if not third_party_notices.is_file():
        raise FileNotFoundError(f"missing staged third-party notices: {third_party_notices}")
    for license_file in project_license_files:
        if not license_file.is_file() or license_file.stat().st_size <= 0:
            raise FileNotFoundError(f"missing staged project license/notice: {license_file}")
    if not user_data_manifest.is_file():
        raise FileNotFoundError(f"missing staged user data manifest: {user_data_manifest}")
    try:
        manifest_data = json.loads(user_data_manifest.read_text(encoding="utf-8"))
        if not isinstance(manifest_data.get("files"), list) or not isinstance(manifest_data.get("directories"), list):
            raise ValueError("files/directories must be arrays")
    except (OSError, ValueError, json.JSONDecodeError) as e:
        raise RuntimeError(f"invalid staged user data manifest: {user_data_manifest} ({e})") from e

    install_dev_text = install_dev.read_text(encoding="utf-8", errors="replace")
    install_current_user_text = install_current_user.read_text(encoding="utf-8", errors="replace")
    uninstall_dev_text = uninstall_dev.read_text(encoding="utf-8", errors="replace")
    uninstall_current_user_text = uninstall_current_user.read_text(encoding="utf-8", errors="replace")
    if "& $RegistrationScript @scriptArgs" in install_dev_text:
        raise RuntimeError(
            "staged install_dev.ps1 directly invokes invoke_registration.ps1; "
            "use a child PowerShell with -ExecutionPolicy Bypass so DllPath is parsed reliably"
        )
    for script_name, script_text in (
        ("install_dev.ps1", install_dev_text),
        ("install_current_user.ps1", install_current_user_text),
    ):
        if "$preserveNames" in script_text:
            raise RuntimeError(
                f"staged {script_name} uses a preserve-list user-state cleanup; "
                "use a transient-name cleanup so future user data is kept by default"
            )
        for transient_name in (
            "cache",
            "logs",
            "install_user.log",
            "install_language_list.log",
        ):
            if transient_name not in script_text:
                raise RuntimeError(
                    f"staged {script_name} does not include known transient cleanup entry: "
                    f"{transient_name}"
                )
        if "preserving all other settings and user data" not in script_text:
            raise RuntimeError(f"staged {script_name} does not document safe user-state cleanup")
        if "Windows rejected TIP after Set-WinUserLanguageList" not in script_text:
            raise RuntimeError(f"staged {script_name} does not verify language-list registration")
        if "zh-CN" not in script_text or "zh-Hans-CN" not in script_text:
            raise RuntimeError(f"staged {script_name} does not handle both zh-CN and zh-Hans-CN")
        if (
            "$EngineRunEntryName" not in script_text
            or "--startup-warmup-delay-ms" not in script_text
        ):
            raise RuntimeError(
                f"staged {script_name} does not register the shared engine for login warmup"
            )
    if "uninstall_machine.log" not in uninstall_dev_text:
        raise RuntimeError("staged uninstall_dev.ps1 does not persist a machine uninstall log")
    if "uninstall_user.log" not in uninstall_current_user_text:
        raise RuntimeError("staged uninstall_current_user.ps1 does not persist a user uninstall log")
    if "RunOnce" not in uninstall_current_user_text:
        raise RuntimeError("staged uninstall_current_user.ps1 does not include current-user cleanup fallback")
    if "Remove-TipFromLoadedUserRegistry" not in uninstall_dev_text:
        raise RuntimeError("staged uninstall_dev.ps1 does not include loaded-user language-list cleanup")
    for script_name, script_text in (
        ("install_dev.ps1", install_dev_text),
        ("install_current_user.ps1", install_current_user_text),
    ):
        if (
            "-ExecutionPolicy', 'Bypass', '-File'" not in script_text
            and "-ExecutionPolicy Bypass -File" not in script_text
        ):
            raise RuntimeError(f"staged {script_name} does not force PowerShell bypass for TIP registration")

    # ---------------------------------------------------------------------------
    skins_dir = output_root / "skins"
    if not skins_dir.is_dir():
        raise FileNotFoundError(f"missing staged skins directory: {skins_dir}")
    theme_count = sum(1 for _ in skins_dir.glob("*/theme.json"))
    if theme_count <= 0:
        raise FileNotFoundError(f"no theme.json under staged skins: {skins_dir}")

# Character table expansion

def collect_artifacts(
    repo: Path,
    profile: str,
    include_installer: bool,
    installer_names: list[str] | None = None,
    rust_artifact_names: list[str] | tuple[str, ...] | None = None,
) -> list[tuple[str, int, str]]:
    """Build helper: collect_artifacts."""
    artifacts: list[tuple[str, int, str]] = []
    profile_cap = profile.capitalize()
    target_dir = rust_target_dir(repo, profile, ARCH_X64)

    # Rust artifacts
    artifact_names = (
        rust_artifact_names
        if rust_artifact_names is not None
        else rust_artifact_names_for_variants(include_smoke_tools=False)
    )
    for name in artifact_names:
        p = target_dir / name
        if p.is_file():
            artifacts.append((name, p.stat().st_size, sha256_prefix(p)))

    # C++ artifacts
    tsf_tip = repo / "tsf-tip"
    for arch in BUILD_ARCHES:
        try:
            tsf_dll = find_tsf_dll(tsf_tip, cmake_build_dir_for(tsf_tip, arch), profile_cap)
            artifacts.append((f"srf_tsf_tip-{arch}.dll", tsf_dll.stat().st_size, sha256_prefix(tsf_dll)))
        except FileNotFoundError:
            pass
        try:
            overlay_exe = find_overlay_exe(
                tsf_tip, cmake_build_dir_for(tsf_tip, arch), profile_cap
            )
            artifacts.append(
                (
                    f"srf_ime_overlay-{arch}.exe",
                    overlay_exe.stat().st_size,
                    sha256_prefix(overlay_exe),
                )
            )
        except FileNotFoundError:
            pass


    # Installer
    for installer_name in installer_names or []:
        exe_path = repo / "dist" / installer_name
        if include_installer and exe_path.is_file():
            artifacts.append((exe_path.name, exe_path.stat().st_size, sha256_prefix(exe_path)))

    return artifacts


def verify_rust_build_artifacts(
    repo: Path,
    profile: str,
    artifact_names: list[str] | tuple[str, ...] | None = None,
) -> None:
    target_dir = rust_target_dir(repo, profile, ARCH_X64)
    names = artifact_names or rust_artifact_names_for_variants()
    for name in names:
        p = target_dir / name
        if p.is_file():
            print_msg(f"  {green('OK')} {name} ({fmt_size(p.stat().st_size)})")
        else:
            raise FileNotFoundError(f"missing Rust build artifact: {p}")


def print_artifact_table(artifacts: list[tuple[str, int, str]]) -> None:
    """Build helper: print_artifact_table."""
    if not artifacts:
        return
    print_msg(f"\n  {'Artifact':<30} {'Size':>12} {'SHA256':>10}")
    print_msg(f"  {'-' * 30} {'-' * 12} {'-' * 10}")
    for name, size, sha in artifacts:
        print_msg(f"  {name:<30} {fmt_size(size):>12} {sha:>10}")


def _print_failure_diagnostics(repo: Path, tsf_tip: Path, profile: str) -> None:
    """Build helper: _print_failure_diagnostics."""
    print_msg(f"\n  {bold(yellow('Diagnostics'))}")
    print_msg(f"  {'-' * 40}")

    # ---------------------------------------------------------------------------
    target_dir = repo / "pinyin-ime" / "target" / profile
    engine_exe = target_dir / "srf_ime_engine.exe"
    if engine_exe.is_file():
        print_msg(f"  Rust engine: {green('exists')} ({fmt_size(engine_exe.stat().st_size)})")
    else:
        print_msg(f"  Rust engine: {red('missing')} (target/{profile}/srf_ime_engine.exe)")

    # ---------------------------------------------------------------------------
    try:
        tsf_dll = find_tsf_dll(tsf_tip, tsf_tip / "build-package", profile.capitalize())
        print_msg(f"  TSF DLL    : {green('exists')} ({fmt_size(tsf_dll.stat().st_size)})")
    except FileNotFoundError:
        print_msg(f"  TSF DLL    : {red('missing')} (CMake may not have built successfully)")

    # ---------------------------------------------------------------------------
    cmake_cache = tsf_tip / "build-package" / "CMakeCache.txt"
    if cmake_cache.is_file():
        print_msg(f"  CMake cache: {green('exists')}")
    else:
        print_msg(f"  CMake cache: {yellow('missing')} (configure may not have run)")

    # ---------------------------------------------------------------------------
    try:
        usage = shutil.disk_usage(repo)
        free_gb = usage.free / (1024 ** 3)
        if free_gb < 2:
            print_msg(f"  Disk free  : {red(f'{free_gb:.1f} GB')} (may be insufficient)")
        else:
            print_msg(f"  Disk free  : {fmt_size(usage.free)}")
    except Exception:
        pass

    print_msg(f"  {'-' * 40}")
    print_msg("  Full log: dist/build-*.log (latest)")
    print_msg("  Try: python build.py --clean")


# ---------------------------------------------------------------------------

def rebuild_stage_zip(output_root: Path) -> Path:
    """Build helper: rebuild_stage_zip."""
    zip_path = output_root.with_suffix(".zip")
    if zip_path.exists():
        zip_path.unlink()
    shutil.make_archive(str(output_root), "zip", root_dir=str(output_root))
    if not zip_path.is_file():
        raise RuntimeError(f"failed to rebuild ZIP: {zip_path}")
    return zip_path


def cleanup_tar_artifacts(dist_dir: Path) -> list[Path]:
    """Remove temporary tar-family archives left in dist by packaging tools."""
    if not dist_dir.is_dir():
        return []

    tar_suffixes = (
        ".tar",
        ".tar.gz",
        ".tgz",
        ".tar.bz2",
        ".tbz",
        ".tbz2",
        ".tar.xz",
        ".txz",
        ".tar.zst",
        ".tzst",
    )
    removed: list[Path] = []
    for path in dist_dir.iterdir():
        if not path.is_file():
            continue
        lower_name = path.name.lower()
        if not lower_name.endswith(tar_suffixes):
            continue
        path.unlink(missing_ok=True)
        removed.append(path)
    return removed


# ---------------------------------------------------------------------------

def _run_check_for_preflight(fn: Callable[[], None]) -> Exception | None:
    try:
        fn()
        return None
    except Exception as exc:
        return exc


def preflight_check(args: argparse.Namespace, repo: Path) -> dict:
    """Build helper: preflight_check."""
    info: dict = {}
    errors: list[str] = []

    print_msg(bold("\n  Preflight"))
    print_msg(f"  {'-' * 40}")

    # Repository layout
    for name in ["pinyin-ime", "tsf-tip", "tsf-tip/installer"]:
        d = repo / name
        if not d.is_dir():
            errors.append(f"missing directory: {d}")
    print_msg(f"  Repo root  : {repo}")
    print_msg(f"  Version    : {green(ctx.version)}")
    if not VERSION_RE.match(ctx.version):
        errors.append(f"invalid VERSION format: {ctx.version!r} (expected like 1.2.3)")
    errors.extend(validate_version_sources(repo))
    if args.no_version and not (args.debug or args.no_inno):
        errors.append("--no-version can only be used with --debug or --no-inno; installer builds must inject VERSION into Inno")

    # UTF-8 and lexicon checks are independent, so run them together.
    check_tasks = [
        ("utf8", lambda: step_check_utf8_sources(repo)),
        ("lexicon", lambda: step_check_lexicon_syllables(repo)),
        ("english", lambda: validate_common_english_lexicon(find_lexicon_dir(repo))),
    ]
    check_errors = (
        [(name, _run_check_for_preflight(fn)) for name, fn in check_tasks]
        if ctx.dry_run
        else run_parallel_tasks(check_tasks, context="preflight")
    )
    for name, exc in check_errors:
        if exc is None:
            continue
        if isinstance(exc, FileNotFoundError):
            errors.append(str(exc))
        elif isinstance(exc, BuildCommandError):
            if name == "utf8":
                detail = "; ".join(exc.tail_lines[-3:]) if exc.tail_lines else "see UTF-8 check output"
                errors.append(f"UTF-8 source encoding check failed: {detail}")
            else:
                detail = "; ".join(exc.tail_lines[-5:]) if exc.tail_lines else "see lexicon syllable check output"
                errors.append(f"lexicon syllable consistency check failed: {detail}")
        else:
            errors.append(f"{name} preflight check failed: {exc}")

    # ---------------------------------------------------------------------------
    skins_root = repo / "skins"
    if skins_root.is_dir():
        theme_count = sum(1 for _ in skins_root.glob("*/theme.json"))
        print_msg(f"  Skins      : {green(str(theme_count))} theme.json")
        if theme_count == 0:
            errors.append(f"no skin theme files found: {skins_root}")
    else:
        errors.append(f"missing directory: {skins_root}")

    # Rust
    if not args.skip_cargo:
        cargo = shutil.which("cargo")
        if cargo:
            try:
                ver = subprocess.check_output([cargo, "--version"], text=True).strip()
                print_msg(f"  Cargo      : {green(ver)}")
            except Exception:
                print_msg(f"  Cargo      : {green(cargo)}")
        elif args.dry_run:
            print_msg(f"  Cargo      : {yellow('not found')} (allowed for --dry-run)")
        else:
            errors.append("cargo not found (install Rust: https://rustup.rs)")

    # CMake
    if not args.skip_cmake:
        cmake = shutil.which("cmake")
        if cmake:
            try:
                ver = subprocess.check_output([cmake, "--version"], text=True).splitlines()[0].strip()
                print_msg(f"  CMake      : {green(ver)}")
            except Exception:
                print_msg(f"  CMake      : {green(cmake)}")
        elif args.dry_run:
            print_msg(f"  CMake      : {yellow('not found')} (allowed for --dry-run)")
        else:
            errors.append("cmake not found (install from https://cmake.org/download/)")

    # PowerShell
    try:
        ps = find_powershell()
        info["powershell"] = ps
        print_msg(f"  PowerShell : {green(str(ps.name))}")
    except FileNotFoundError as e:
        if args.dry_run:
            info["powershell"] = Path("powershell.exe")
            print_msg(f"  PowerShell : {yellow('not found')} (allowed for --dry-run)")
        else:
            errors.append(str(e))

    # Inno Setup
    if not args.no_inno and not args.debug:
        iscc = find_iscc()
        if iscc:
            print_msg(f"  Inno Setup : {green(str(iscc))}")
        elif args.dry_run:
            print_msg(f"  Inno Setup : {yellow('not found')} (allowed for --dry-run)")
        else:
            errors.append("ISCC.exe not found (install Inno Setup 6, or use --no-inno)")

    if signing_requested(args):
        signtool = find_signtool()
        has_cert = bool(os.environ.get("KX_SIGN_CERT_SHA1") or os.environ.get("KX_SIGN_PFX"))
        if signtool and has_cert:
            print_msg(f"  SignTool   : {green(str(signtool))}")
        elif args.sign_required:
            errors.append("code signing is required but signtool or KX_SIGN_CERT_SHA1/KX_SIGN_PFX is missing")
        else:
            print_msg(f"  SignTool   : {yellow('skipped')} (set KX_SIGN_CERT_SHA1 or KX_SIGN_PFX)")

    try:
        lex = find_lexicon_dir(repo)
        info["lexicon"] = lex
        source_count = len(_lexicon_source_files(repo))
        print_msg(f"  Lexicon    : {green(str(lex.name))} ({source_count} source files)")
        english_count = validate_common_english_lexicon(lex)
        print_msg(f"  English    : {green(str(english_count))} validated words")
    except FileNotFoundError as e:
        errors.append(str(e))
    except RuntimeError as e:
        errors.append(str(e))

    if errors:
        print_msg(f"\n  {red('ERROR')} preflight failed")
        for e in errors:
            print_msg(f"    {red('x')} {e}")
        raise RuntimeError("preflight failed")

    print_msg(f"  {'-' * 40}")
    print_msg(f"  {green('OK')} preflight passed\n")
    return info


# ---------------------------------------------------------------------------

def parse_args() -> argparse.Namespace:
    p = argparse.ArgumentParser(
        description="开心输入法 build script",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="Examples:\n"
               "  python build.py              full Release build\n"
               "  python build.py --package-variants full\n"
               "                               complete installer (IME + OCR)\n"
               "  python build.py --debug      Debug build\n"
               "  python build.py --quick      stage only\n"
               "  python build.py --clean      clean rebuild\n"
               "  python build.py --no-verify  skip Rust/C++ correctness gates\n"
               "  python build.py --perf-smoke build and enforce input P99 gate\n"
               "  python build.py --quiet      quiet mode\n"
               "  python build.py --dry-run    print commands only\n",
    )
    p.add_argument("--debug", action="store_true",
                   help="use Debug configuration (default: Release)")
    p.add_argument("--clean", action="store_true",
                   help="clean build caches before building")
    p.add_argument("--quick", action="store_true",
                   help="skip cargo and cmake; only stage/package")
    p.add_argument("--skip-cargo", action="store_true",
                   help="skip Rust cargo build; for staging/dev only, never for official packaging")
    p.add_argument("--skip-cmake", action="store_true",
                   help="skip C++ cmake build")
    p.add_argument("--no-inno", action="store_true",
                   help="skip Inno Setup installer generation")
    p.add_argument("--user-installer", action="store_true",
                   help="generate per-user installers instead of machine-wide installers")
    p.add_argument("--rebuild-corpus", action="store_true",
                   help="force rebuild corpus.txt")
    p.add_argument("--regenerate-lexicons", action="store_true",
                   help="regenerate native lexicons and pinyin_supported_chars.txt before building")
    p.add_argument("--portable-zip", "--zip", dest="portable_zip", action="store_true",
                   help="also generate portable ZIP artifacts")
    p.set_defaults(portable_zip=False)
    p.add_argument("--package-variants", default="all",
                   help=(
                       "comma-separated variants to package: all, ime, ocr, "
                       "full (full is an alias for ocr; default: all)"
                   ))
    p.add_argument("--include-translation", dest="include_translation", action="store_true",
                   help="deprecated compatibility option; external WinTranslator linkage is always retained")
    p.add_argument("--no-translation", dest="include_translation", action="store_false",
                   help="deprecated compatibility option; no translation runtime or model is packaged")
    p.set_defaults(include_translation=None)
    p.add_argument("--quiet", "-q", action="store_true",
                   help="quiet mode")
    p.add_argument("--verbose", "-v", action="store_true",
                   help="verbose mode")
    p.add_argument("--dry-run", action="store_true",
                   help="print commands without running them")
    p.add_argument("--force", action="store_true",
                   help="force rebuild, ignoring incremental checks")
    p.add_argument("--no-parallel", action="store_true",
                   help="disable parallel Rust/C++ build")
    p.add_argument("--no-smoke", action="store_true",
                   help="skip post-build smoke checks")
    p.add_argument("--verify", dest="verify", action="store_true",
                   help="run Rust and C++ correctness checks (default for non-quick builds)")
    p.add_argument("--no-verify", dest="verify", action="store_false",
                   help="skip Rust and C++ correctness gates")
    p.set_defaults(verify=None)
    p.add_argument("--perf-smoke", action="store_true",
                   help="run release input_perf after compilation and enforce a P99 gate")
    p.add_argument("--perf-max-p99-us", type=int, default=DEFAULT_PERF_MAX_P99_US,
                   help=f"maximum input_perf P99 in microseconds (default: {DEFAULT_PERF_MAX_P99_US})")
    p.add_argument(
        "--perf-max-incremental-p99-us",
        type=int,
        default=DEFAULT_PERF_INCREMENTAL_MAX_P99_US,
        help=(
            "maximum zero-warmup incremental input_perf P99 in microseconds "
            f"(default: {DEFAULT_PERF_INCREMENTAL_MAX_P99_US})"
        ),
    )
    p.add_argument("--no-skin-check", action="store_true",
                   help="skip skins/*/theme.json validation")
    p.add_argument("--collect-runtime-logs", action="store_true",
                   help="copy runtime logs from %%LOCALAPPDATA%%\\kaixin into dist/runtime-logs")
    p.add_argument("--no-version", action="store_true",
                   help="do not pass /DKXAppVersion to ISCC; only useful with --debug or --no-inno")
    p.add_argument("--sign", action="store_true",
                   help="sign staged binaries and installers when signtool and KX_SIGN_* environment variables are available")
    p.add_argument("--sign-required", action="store_true",
                   help="fail the build if code signing is unavailable or fails")
    return p.parse_args()


def main() -> int:
    _ensure_stdout_utf8()

    if sys.platform != "win32":
        print(red("error: this script only supports Windows."))
        return 1

    args = parse_args()
    # ``ctx`` is intentionally shared by helper functions, but each CLI
    # invocation must start with a fresh timestamp, timings and log state.
    ctx.reset_run_state()
    if args.quick:
        args.skip_cargo = True
        args.skip_cmake = True
    if args.verify is None:
        args.verify = not args.skip_cargo
    if args.verify and args.skip_cargo:
        print(red("error: --verify cannot be combined with --quick/--skip-cargo"))
        return 2
    if args.perf_smoke and args.debug:
        print(red("error: --perf-smoke requires a Release build (remove --debug)"))
        return 2
    if args.perf_max_p99_us <= 0:
        print(red("error: --perf-max-p99-us must be greater than zero"))
        return 2
    if args.perf_max_incremental_p99_us <= 0:
        print(red("error: --perf-max-incremental-p99-us must be greater than zero"))
        return 2

    # ---------------------------------------------------------------------------
    ctx.quiet = args.quiet and not args.verbose
    ctx.verbose = args.verbose
    ctx.dry_run = args.dry_run
    ctx.no_version = args.no_version

    repo = resolve_repo_root()
    profile = "debug" if args.debug else "release"
    profile_cap = profile.capitalize()
    runtime_logs_dirs = runtime_log_roots()
    tsf_tip = repo / "tsf-tip"
    cmake_build_dir = cmake_build_dir_for(tsf_tip, ARCH_X64)
    cmake_build_dir_x86 = cmake_build_dir_for(tsf_tip, ARCH_X86)
    ctx.generator_cache_file = cmake_build_dir / ".cmake_generator_cache"
    ocr_ready, ocr_problems = rapidocr_runtime_status(repo)
    # Both packages retain the lightweight WinTranslator linkage. Translation
    # models and runtimes belong to the separately installed app and are never staged here.
    try:
        package_variants, skipped_package_variants = resolve_package_variants(
            args, ocr_ready
        )
    except (ValueError, RuntimeError) as e:
        print(red(f"error: {e}"))
        for problem in ocr_problems:
            print(f"  - {problem}")
        return 1
    stage_roots = [stage_root_for_variant(repo, variant) for variant in package_variants]
    primary_variant = package_variants[-1]
    output_root = stage_root_for_variant(repo, primary_variant)
    args.package_variant_ids = [variant.id for variant in package_variants]
    args.include_ocr_resolved = any(variant.include_ocr for variant in package_variants)
    args.include_translation_resolved = any(variant.include_translation for variant in package_variants)
    required_rust_artifacts = rust_artifact_names_for_variants(
        package_variants,
        include_smoke_tools=not args.no_smoke,
    )
    if args.perf_smoke and PERF_RUST_ARTIFACT not in required_rust_artifacts:
        required_rust_artifacts.append(PERF_RUST_ARTIFACT)
    installer_output_by_variant = {
        variant.id: timestamped_installer_name(
            ctx.version,
            ctx.build_timestamp,
            user_installer=bool(args.user_installer),
            variant_suffix=variant.installer_suffix,
        )
        for variant in package_variants
    }
    args.installer_output_names = list(installer_output_by_variant.values())

    dist_dir = repo / "dist"
    if not ctx.dry_run:
        dist_dir.mkdir(parents=True, exist_ok=True)
        # ---------------------------------------------------------------------------
        _cleanup_old_logs(dist_dir)
        log_name = f"build-{ctx.build_timestamp}.log"
        log_path = dist_dir / log_name
        try:
            ctx.log_file = open(log_path, "w", encoding="utf-8")
        except Exception:
            ctx.log_file = None
    else:
        log_path = dist_dir / "build-dry-run.log"

    total_start = time.monotonic()

    # Build steps
    print_msg()
    print_msg(bold(cyan("  开心输入法 Build")))
    print_msg(f"  Version: {bold(ctx.version)}  |  Profile: {bold(profile_cap)}  |  Time: {datetime.now().strftime('%Y-%m-%d %H:%M:%S')}")
    print_msg("  Package variants: " + ", ".join(bold(variant.label) for variant in package_variants))
    if skipped_package_variants:
        print_msg(
            f"  {yellow('!')} skipped package variants: "
            + ", ".join(variant.label for variant in skipped_package_variants)
        )
    if not ocr_ready and any(variant.include_ocr for variant in skipped_package_variants):
        print_msg(f"  {yellow('!')} local RapidOCR runtime is invalid; OCR variants will be skipped")
        for problem in ocr_problems:
            print_msg(f"    {yellow('!')} {problem}")
    #
    #
    if ctx.dry_run:
        print_msg(yellow("  *** DRY RUN: commands are printed but not executed ***"))

    try:
        # Keep preflight failures on the same diagnostics and log-cleanup path
        # as failures from the actual build steps.
        info = preflight_check(args, repo)
        powershell = info["powershell"]

        # ---------------------------------------------------------------------------
        if args.quick and not ctx.dry_run:
            require_fresh_rust_artifacts(
                repo,
                profile,
                "--quick packaging",
                required_artifacts=required_rust_artifacts,
            )
            missing: list[str] = []
            # Rust artifacts
            target_dir = rust_target_dir(repo, profile, ARCH_X64)
            for name in required_rust_artifacts:
                if not (target_dir / name).is_file():
                    missing.append(f"pinyin-ime/target/{profile}/{name}")

            # C++ artifacts
            try:
                _ = find_tsf_runtime_pair(tsf_tip, cmake_build_dir, profile_cap)
            except FileNotFoundError:
                missing.append(f"tsf-tip/build-package/{profile_cap}/srf_tsf_tip.dll")
                missing.append(f"tsf-tip/build-package/{profile_cap}/srf_ime_overlay.exe")
            try:
                _ = find_tsf_runtime_pair(tsf_tip, cmake_build_dir_x86, profile_cap)
            except FileNotFoundError:
                missing.append(f"tsf-tip/build-package-x86/{profile_cap}/srf_tsf_tip.dll")
                missing.append(
                    f"tsf-tip/build-package-x86/{profile_cap}/srf_ime_overlay.exe"
                )

            if missing:
                print_msg(f"\n  {red('FAIL')} --quick requires existing build artifacts; missing:")
                for m in missing:
                    print_msg(f"    {red('x')} {m}")
                print_msg("\n  Suggestion: run a full build first:")
                print_msg(f"    python build.py{' --debug' if args.debug else ''}")
                print_msg("\n  Or rebuild only the missing side:")
                print_msg("    python build.py --skip-cmake    or    python build.py --skip-cargo")
                return 1

        # Parallel build
        if args.clean:
            with StepTimer("Step 0/5  Clean build caches"):
                targets = [
                    repo / "pinyin-ime" / "target",
                    cmake_build_dir,
                    cmake_build_dir_x86,
                    repo / "dist" / "kaixin-package",
                    *[stage_root_for_variant(repo, variant) for variant in PACKAGE_VARIANTS.values()],
                    repo / "pinyin-ime" / "data" / "corpus.txt",
                ]
                for t in targets:
                    if t.exists():
                        if ctx.dry_run:
                            print_msg(f"  [dry-run] delete: {t}")
                        else:
                            print_msg(f"  delete: {t}")
                            shutil.rmtree(t, ignore_errors=True) if t.is_dir() else t.unlink(missing_ok=True)

                # ---------------------------------------------------------------------------
                purged = purge_lexicon_caches(repo, output_root=None)
                for p in purged:
                    if ctx.dry_run:
                        print_msg(f"  [dry-run] delete: {p}")
                    else:
                        print_msg(f"  delete: {p}")
                print_msg(f"  {green('OK')} clean {'dry-run ' if ctx.dry_run else ''}complete")

        # Smoke validation
        with StepTimer("Step 1/5  Prepare corpus"):
            if ctx.dry_run:
                print_msg("  [dry-run] generate corpus.txt")
            else:
                ensure_corpus(repo, force=args.rebuild_corpus)

        if not args.no_skin_check:
            with StepTimer("Step 1a/5  Validate skins"):
                skin_warnings = step_validate_skins(repo)
                for w in skin_warnings:
                    print_msg(f"  {yellow('!')} skin: {w}")

        # Environment checks
        if args.regenerate_lexicons:
            with StepTimer("Step 1b/5  Regenerate clean lexicons"):
                step_regenerate_lexicons(repo)

        if args.verify:
            with StepTimer("Step 1c/5  Verify Rust format and checks"):
                step_verify_rust(repo)
        else:
            print_msg(f"\n{dim('  [skip] Step 1c/5  Rust verification')}")

        # ---------------------------------------------------------------------------
        skip_cargo_inc = False
        skip_cmake_inc = False
        skip_cmake_x86_inc = False
        # Cargo and CMake are independent products.  Check each one even when
        # the other component was explicitly skipped; otherwise ``--skip-cmake``
        # needlessly rebuilds Rust (and vice versa), defeating incremental use
        # in focused developer builds.
        if not args.force and not args.clean:
            if not args.skip_cargo and is_cargo_up_to_date(
                repo,
                profile,
                ARCH_X64,
                required_artifacts=required_rust_artifacts,
            ):
                print_msg(f"  {dim('>> Rust artifacts are up to date; skipping build (--force to rebuild)')}")
                skip_cargo_inc = True
            if not args.skip_cmake and is_cmake_up_to_date(repo, tsf_tip, profile, ARCH_X64):
                print_msg(f"  {dim('>> C++ artifacts are up to date; skipping build (--force to rebuild)')}")
                skip_cmake_inc = True
            if not args.skip_cmake and is_cmake_up_to_date(repo, tsf_tip, profile, ARCH_X86):
                print_msg(f"  {dim('>> C++ x86 TIP is up to date; skipping build (--force to rebuild)')}")
                skip_cmake_x86_inc = True

        do_cargo = not args.skip_cargo and not skip_cargo_inc
        do_cmake = not args.skip_cmake and not skip_cmake_inc
        do_cmake_x86 = not args.skip_cmake and not skip_cmake_x86_inc
        cmake_x86_built_in_parallel = False

        # ---------------------------------------------------------------------------
        if do_cargo and do_cmake and not args.no_parallel:
            build_x86_in_parallel = do_cmake_x86
            title = (
                f"Step 2+3/5  Build Rust + C++ x64/x86 ({profile_cap})"
                if build_x86_in_parallel
                else f"Step 2+3/5  Build Rust + C++ ({profile_cap})"
            )
            with StepTimer(title):
                step_cargo_and_cmake_parallel(
                    repo,
                    tsf_tip,
                    cmake_build_dir,
                    profile,
                    clean=args.clean,
                    cmake_build_dir_x86=cmake_build_dir_x86,
                    build_x86=build_x86_in_parallel,
                    rust_artifact_names=required_rust_artifacts,
                )
                if not ctx.dry_run:
                    # Verify artifacts
                    verify_rust_build_artifacts(
                        repo,
                        profile,
                        required_rust_artifacts,
                    )
                    dll = find_tsf_dll(tsf_tip, cmake_build_dir, profile_cap)
                    print_msg(f"  {green('OK')} srf_tsf_tip.dll ({fmt_size(dll.stat().st_size)})")
                    if build_x86_in_parallel:
                        x86_dll = find_tsf_dll(tsf_tip, cmake_build_dir_x86, profile_cap)
                        assert_pe_arch(x86_dll, ARCH_X86, "x86 srf_tsf_tip.dll")
                        print_msg(f"  {green('OK')} srf_tsf_tip.dll x86 ({fmt_size(x86_dll.stat().st_size)})")
                if build_x86_in_parallel:
                    cmake_x86_built_in_parallel = True
                    do_cmake_x86 = False
        else:
            if do_cargo:
                with StepTimer(f"Step 2/5  Build Rust engine ({profile_cap})"):
                    step_cargo(repo, profile, artifact_names=required_rust_artifacts)
                    if not ctx.dry_run:
                        verify_rust_build_artifacts(
                            repo,
                            profile,
                            required_rust_artifacts,
                        )
            elif skip_cargo_inc:
                print_msg(f"\n{dim('  [skip] Step 2/5  Rust build (up to date)')}")
            else:
                print_msg(f"\n{dim('  [skip] Step 2/5  Rust build')}")

            if do_cmake:
                with StepTimer(f"Step 3/5  Build C++ TSF module ({profile_cap})"):
                    step_cmake(repo, tsf_tip, cmake_build_dir, profile, clean=args.clean)
                    if not ctx.dry_run:
                        dll = find_tsf_dll(tsf_tip, cmake_build_dir, profile_cap)
                        print_msg(f"  {green('OK')} srf_tsf_tip.dll ({fmt_size(dll.stat().st_size)})")
            elif skip_cmake_inc:
                print_msg(f"\n{dim('  [skip] Step 3/5  CMake build (up to date)')}")
            else:
                print_msg(f"\n{dim('  [skip] Step 3/5  CMake build')}")

        if cmake_x86_built_in_parallel:
            pass
        elif do_cmake_x86:
            with StepTimer(f"Step 3a/5  Build C++ TSF module x86 ({profile_cap})"):
                step_cmake(repo, tsf_tip, cmake_build_dir_x86, profile, clean=args.clean, arch=ARCH_X86)
                if not ctx.dry_run:
                    dll = find_tsf_dll(tsf_tip, cmake_build_dir_x86, profile_cap)
                    assert_pe_arch(dll, ARCH_X86, "x86 srf_tsf_tip.dll")
                    print_msg(f"  {green('OK')} srf_tsf_tip.dll x86 ({fmt_size(dll.stat().st_size)})")
        elif skip_cmake_x86_inc:
            print_msg(f"\n{dim('  [skip] Step 3a/5  CMake x86 build (up to date)')}")
        else:
            print_msg(f"\n{dim('  [skip] Step 3a/5  CMake x86 build')}")

        if not args.no_smoke and not args.dry_run:
            warnings = smoke_verify(repo, profile)
            if warnings:
                print_msg(f"\n  {yellow('! smoke warnings:')}")
                for w in warnings:
                    print_msg(f"    {yellow('!')} {w}")

        if args.perf_smoke:
            with StepTimer(
                "Step 3c/5  Release input performance smoke "
                f"(warm P99 <= {args.perf_max_p99_us}us; "
                f"incremental P99 <= {args.perf_max_incremental_p99_us}us)"
            ):
                if args.skip_cargo and not ctx.dry_run:
                    require_fresh_rust_artifacts(
                        repo,
                        profile,
                        "performance smoke",
                        required_artifacts=required_rust_artifacts,
                    )
                step_input_perf_smoke(
                    repo,
                    profile,
                    args.perf_max_p99_us,
                    args.perf_max_incremental_p99_us,
                )

        with StepTimer("Step 4/5  Stage packages"):
            # ---------------------------------------------------------------------------
            if args.skip_cargo and not ctx.dry_run:
                require_fresh_rust_artifacts(
                    repo,
                    profile,
                    "staging",
                    required_artifacts=required_rust_artifacts,
                )
            if ctx.dry_run:
                try:
                    tip_dll, overlay_exe = find_tsf_runtime_pair(
                        tsf_tip, cmake_build_dir, profile_cap
                    )
                except FileNotFoundError:
                    tip_dll = cmake_build_dir / profile_cap / "srf_tsf_tip.dll"
                    overlay_exe = cmake_build_dir / profile_cap / "srf_ime_overlay.exe"
                try:
                    x86_tip_dll, x86_overlay_exe = find_tsf_runtime_pair(
                        tsf_tip, cmake_build_dir_x86, profile_cap
                    )
                except FileNotFoundError:
                    x86_tip_dll = cmake_build_dir_x86 / profile_cap / "srf_tsf_tip.dll"
                    x86_overlay_exe = (
                        cmake_build_dir_x86 / profile_cap / "srf_ime_overlay.exe"
                    )
            else:
                tip_dll, overlay_exe = find_tsf_runtime_pair(
                    tsf_tip, cmake_build_dir, profile_cap
                )
                x86_tip_dll, x86_overlay_exe = find_tsf_runtime_pair(
                    tsf_tip, cmake_build_dir_x86, profile_cap
                )
                assert_pe_arch(tip_dll, ARCH_X64, "x64 srf_tsf_tip.dll")
                assert_pe_arch(overlay_exe, ARCH_X64, "x64 srf_ime_overlay.exe")
                assert_pe_arch(x86_tip_dll, ARCH_X86, "x86 srf_tsf_tip.dll")
                assert_pe_arch(
                    x86_overlay_exe, ARCH_X86, "x86 srf_ime_overlay.exe"
                )
            make_zip = bool(args.portable_zip)
            for variant in package_variants:
                stage_root = stage_root_for_variant(repo, variant)
                purged = purge_lexicon_caches(repo, output_root=stage_root)
                if purged:
                    for p in purged:
                        tag = "[dry-run] delete" if ctx.dry_run else "delete"
                        print_msg(f"  {tag}: {p}")
                print_msg(f"  Stage variant: {variant.label} -> {stage_root}")
                step_stage(repo, tsf_tip, tip_dll, overlay_exe, x86_tip_dll,
                           x86_overlay_exe, stage_root,
                           powershell, make_zip=make_zip, profile_cap=profile_cap,
                           include_ocr=variant.include_ocr,
                           include_translation=variant.include_translation)
                if not ctx.dry_run and stage_root.is_dir():
                    verify_staged_package(
                        stage_root,
                        include_ocr=variant.include_ocr,
                        include_translation=variant.include_translation,
                        repo=repo,
                    )
                    total = sum(f.stat().st_size for f in stage_root.rglob("*") if f.is_file())
                    print_msg(f"  {green('OK')} stage directory: {stage_root}")
                    print_msg(f"  {green('OK')} total size: {fmt_size(total)}")
                    print_msg(f"  {green('OK')} staged package verification passed")

                    if not args.no_smoke and variant.id == primary_variant.id:
                        warnings = post_stage_smoke_verify(repo, stage_root, profile)
                        if warnings:
                            print_msg(f"\n  {yellow('! post-stage smoke warnings:')}")
                            for w in warnings:
                                print_msg(f"    {yellow('!')} {w}")

                    if signing_requested(args):
                        signed = sign_staged_binaries(stage_root, args)
                        if signed:
                            print_msg(f"  {green('OK')} signed staged binaries: {len(signed)}")

                    stage_artifacts = collect_artifacts(
                        repo,
                        profile,
                        include_installer=False,
                        installer_names=[],
                        rust_artifact_names=rust_artifact_names_for_variants(
                            [variant],
                            include_smoke_tools=False,
                        ),
                    )
                    write_build_manifest(
                        repo,
                        stage_root,
                        profile,
                        args,
                        stage_artifacts,
                        extra_output_roots=[],
                        copy_to_stage=True,
                        include_installer_files=False,
                        package_variant=variant,
                    )
                    print_msg(f"  {green('OK')} staged build manifest: build-manifest.json")

                    package_manifest = write_package_hash_manifest(stage_root)
                    write_component_manifest_from_package_manifest(stage_root, package_manifest)
                    verified = verify_package_hash_manifest(stage_root)
                    print_msg(
                        f"  {green('OK')} package integrity manifest: "
                        f"{package_manifest.name} ({verified} files)"
                    )

                    if make_zip:
                        zip_path = rebuild_stage_zip(stage_root)
                        suffix = f"-{variant.installer_suffix}"
                        portable_name = f"kaixin-portable{suffix}-{ctx.version}-{profile_cap}-{datetime.now().strftime('%Y%m%d-%H%M%S')}.zip"
                        portable_zip = zip_path.parent / portable_name
                        stable_portable_zip = zip_path.parent / f"kaixin-portable{suffix}.zip"
                        shutil.copy2(zip_path, portable_zip)
                        shutil.copy2(zip_path, stable_portable_zip)
                        print_msg(f"  {green('OK')} portable ZIP: {portable_zip} ({fmt_size(portable_zip.stat().st_size)})")
                        print_msg(f"  {green('OK')} stable portable ZIP: {stable_portable_zip} ({fmt_size(stable_portable_zip.stat().st_size)})")

        if not args.no_inno and not args.debug:
            with StepTimer("Step 5/5  Generate installer (Inno Setup)"):
                if not ctx.dry_run:
                    removed_installers = cleanup_existing_installer_exes(dist_dir)
                    for removed in removed_installers:
                        print_msg(f"  delete old installer: {removed.name}")
                installer_script = "kaixin-user.iss" if args.user_installer else "kaixin.iss"
                for variant in package_variants:
                    installer_output_name = installer_output_by_variant[variant.id]
                    exe = step_inno(
                        repo,
                        script_name=installer_script,
                        output_name=installer_output_name,
                        package_dir=stage_root_for_variant(repo, variant),
                        include_ocr=variant.include_ocr,
                        include_translation=variant.include_translation,
                    )
                    if not ctx.dry_run:
                        sign_file(exe, args)
                        stat = exe.stat()
                        print_msg(f"  {green('OK')} {variant.label}: {exe.name}")
                        print_msg(f"  {green('OK')} size: {fmt_size(stat.st_size)}")
        else:
            reason = "Debug" if args.debug else "--no-inno"
            print_msg(f"\n{dim(f'  [skip] Step 5/5  Inno Setup ({reason})')}")

    except BuildCommandError as e:
        print(f"\n  {red('FAIL')} {e}", file=sys.stderr)
        if ctx.log_file:
            ctx.log_file.write(f"\nFAIL: {e}\n")
        _print_failure_diagnostics(repo, tsf_tip, profile)
        for runtime_logs_dir in runtime_logs_dirs:
            print_msg(f"  Runtime logs: {runtime_logs_dir}")
        if ctx.log_file:
            ctx.log_file.close()
            ctx.log_file = None
            print(f"  Build log: {log_path}", file=sys.stderr)
        return e.returncode or 1
    except subprocess.CalledProcessError as e:
        print(f"\n  {red('FAIL')} command failed (exit {e.returncode})", file=sys.stderr)
        _print_failure_diagnostics(repo, tsf_tip, profile)
        for runtime_logs_dir in runtime_logs_dirs:
            print_msg(f"  Runtime logs: {runtime_logs_dir}")
        if ctx.log_file:
            ctx.log_file.close()
            ctx.log_file = None
            print(f"  Build log: {log_path}", file=sys.stderr)
        return e.returncode or 1
    except (FileNotFoundError, RuntimeError) as e:
        print(f"\n  {red('FAIL')} {e}", file=sys.stderr)
        if args.skip_cargo:
            print_msg(
                f"  {yellow('!')} If this was meant to be a formal package, rerun without --skip-cargo "
                "so Rust artifacts are rebuilt and restaged."
            )
        for runtime_logs_dir in runtime_logs_dirs:
            print_msg(f"  Runtime logs: {runtime_logs_dir}")
        if ctx.log_file:
            ctx.log_file.close()
            ctx.log_file = None
            print(f"  Build log: {log_path}", file=sys.stderr)
        return 1

    total_elapsed = time.monotonic() - total_start

    if ctx.dry_run:
        print_msg(f"\n{'=' * 60}")
        print_msg(bold(yellow("  DRY RUN COMPLETE (no commands were executed)")))
        print_msg(f"{'=' * 60}")
        print_msg(f"  Version: {ctx.version}  |  Profile: {profile_cap}  |  Dry-run elapsed: {fmt_elapsed(total_elapsed)}")
        print_msg()
        return 0

    installer_expected = not args.no_inno and not args.debug
    artifacts = collect_artifacts(
        repo,
        profile,
        include_installer=installer_expected,
        installer_names=list(getattr(args, "installer_output_names", [])),
        rust_artifact_names=rust_artifact_names_for_variants(
            package_variants,
            include_smoke_tools=False,
        ),
    )
    manifest_path = write_build_manifest(
        repo,
        output_root,
        profile,
        args,
        artifacts,
        extra_output_roots=[root for root in stage_roots if root != output_root],
        copy_to_stage=False,
        include_installer_files=installer_expected,
    )
    # Every staged variant's package hash manifest was already written and
    # verified during Step 4 (write + verify per variant); nothing modifies the
    # stage directories afterwards, so re-verifying here would only run a third
    # full SHA-256 pass over the same bytes (most costly on the OCR payload).
    artifacts.append((manifest_path.name, manifest_path.stat().st_size, sha256_prefix(manifest_path)))
    ctx.artifacts = artifacts

    print_msg(f"\n{'=' * 60}")
    print_msg(bold(green("  BUILD SUCCESS")))
    print_msg(f"{'=' * 60}")
    print_msg(f"  Version: {ctx.version}  |  Profile: {profile_cap}  |  Elapsed: {fmt_elapsed(total_elapsed)}")

    # Step timing summary
    if ctx.step_results:
        print_msg(f"\n  Step timings:")
        for title, elapsed in ctx.step_results.items():
            label = re.sub(r"^\s*Step \d+[a-z]?(?:\+\d+)?/\d+\s*", "", title, flags=re.IGNORECASE)
            print_msg(f"    {label:<40} {fmt_elapsed(elapsed):>8}")

    print_artifact_table(artifacts)

    installer_paths = [
        repo / "dist" / name
        for name in getattr(args, "installer_output_names", [])
    ]
    existing_installers = [path for path in installer_paths if path.is_file()]
    if installer_expected and existing_installers:
        print_msg(f"\n  Installers:")
        label_by_installer = {
            installer_output_by_variant[variant.id]: variant.label
            for variant in package_variants
        }
        for exe_path in existing_installers:
            st = exe_path.stat()
            mtime = datetime.fromtimestamp(st.st_mtime).strftime("%Y-%m-%d %H:%M:%S")
            label = label_by_installer.get(exe_path.name, "installer")
            print_msg(f"    - {label}: {bold(str(exe_path))} ({fmt_size(st.st_size)}, {mtime})")
    else:
        print_msg(f"\n  Stage directories:")
        for variant, stage_root in zip(package_variants, stage_roots):
            print_msg(f"    - {variant.label}: {stage_root}")
        if args.debug:
            print_msg(f"  (Debug mode does not generate an installer)")
        elif args.no_inno:
            print_msg(f"  (Installer generation was skipped; run ISCC manually if needed)")

    if args.portable_zip:
        for zip_path in (stage_root.with_suffix(".zip") for stage_root in stage_roots):
            if zip_path.is_file():
                print_msg(f"  ZIP: {zip_path} ({fmt_size(zip_path.stat().st_size)})")

    if args.skip_cargo:
        print_skip_cargo_packaging_warning()

    print_msg("\n  Runtime logs:")
    if not runtime_logs_dirs:
        print_msg("    (LOCALAPPDATA is not available in this environment)")
    else:
        for runtime_logs_dir in runtime_logs_dirs:
            print_msg(f"    Source: {runtime_logs_dir}")
            for name in known_runtime_logs():
                print_msg(f"    - {runtime_logs_dir / name}")

    if args.collect_runtime_logs:
        copied_logs = collect_runtime_logs(repo)
        if copied_logs:
            print_msg(f"  Collected runtime logs: {repo / 'dist' / 'runtime-logs'}")
            for path in copied_logs:
                print_msg(f"    - {path}")
        else:
            print_msg("  Collected runtime logs: none found")

    removed_tar_artifacts = cleanup_tar_artifacts(dist_dir)
    if removed_tar_artifacts:
        print_msg("\n  Removed tar build artifacts:")
        for path in removed_tar_artifacts:
            print_msg(f"    - {path}")

    # Build log path
    if ctx.log_file:
        ctx.log_file.close()
        ctx.log_file = None
        print_msg(f"\n  Build log: {log_path}")

    print_msg()
    return 0


if __name__ == "__main__":
    sys.exit(main())
