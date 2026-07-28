#!/usr/bin/env python3
"""Validate the staged package manifest when a staged package is present."""

from __future__ import annotations

import json
import hashlib
import configparser
import re
import sys
import zipfile
from pathlib import Path, PurePosixPath


PACKAGE_HASH_MANIFEST = "package_manifest.sha256"
COMPONENT_MANIFEST = "component_manifest.ini"
SHAREX_SOURCE_ARCHIVE_NAME = "kaixin-sharex-corresponding-source.zip"
SHAREX_SOURCE_NOTICE_MARKER = (
    f"distributed separately as `{SHAREX_SOURCE_ARCHIVE_NAME}`"
)
PYTHON_EXCLUDED_DIR_NAMES = {
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
PYTHON_EXCLUDED_FILE_SUFFIXES = {
    ".pyc",
    ".pyo",
    ".pdb",
    ".lib",
    ".exp",
    ".h",
    ".hpp",
    ".c",
    ".cpp",
    ".cxx",
    ".pxd",
    ".pyi",
}
REQUIRED_FILES = {
    "srf_ime_engine.exe",
    "srf_ime_tray.exe",
    "srf_ime_settings.exe",
    "install_dev.ps1",
    "install_current_user.ps1",
    "invoke_registration.ps1",
    "current_runtime_payload.txt",
    PACKAGE_HASH_MANIFEST,
    COMPONENT_MANIFEST,
    "build-manifest.json",
}

COMPONENT_PREFIXES = {
    "sharex": "ShareX/",
    "python_runtime": ".python-runtime/",
    "rapidocr_venv": ".venv-rapidocr/",
    "rapidocr_payload": "RapidOCR-3.9.0/",
}


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def manifest_relative_path(root: Path, rel: str) -> Path:
    if "\\" in rel:
        raise ValueError(f"unsafe package manifest path: {rel}")
    rel_path = PurePosixPath(rel)
    if rel_path.is_absolute() or any(part in ("", "..") or ":" in part for part in rel_path.parts):
        raise ValueError(f"unsafe package manifest path: {rel}")
    target = (root / Path(*rel_path.parts)).resolve()
    try:
        target.relative_to(root.resolve())
    except ValueError as exc:
        raise ValueError(f"package manifest path escapes staged package root: {rel}") from exc
    return target


def validate_package_hash_manifest(package: Path, errors: list[str]) -> None:
    manifest_path = package / PACKAGE_HASH_MANIFEST
    if not manifest_path.is_file():
        return

    listed: set[str] = set()
    verified = 0
    try:
        lines = manifest_path.read_text(encoding="utf-8-sig").splitlines()
    except Exception as exc:
        errors.append(f"invalid {PACKAGE_HASH_MANIFEST}: {exc}")
        return

    for line_no, line in enumerate(lines, 1):
        trimmed = line.strip()
        if not trimmed or trimmed.startswith("#"):
            continue
        match = re.fullmatch(r"([0-9a-fA-F]{64}) \*(.+)", trimmed)
        if not match:
            errors.append(f"{PACKAGE_HASH_MANIFEST}:{line_no}: invalid line: {trimmed}")
            continue
        expected = match.group(1).lower()
        rel = match.group(2)
        try:
            path = manifest_relative_path(package, rel)
        except ValueError as exc:
            errors.append(str(exc))
            continue
        if not path.is_file():
            errors.append(f"{PACKAGE_HASH_MANIFEST}:{line_no}: missing file: {rel}")
            continue
        actual = sha256_file(path)
        if actual != expected:
            errors.append(f"{PACKAGE_HASH_MANIFEST}:{line_no}: SHA-256 mismatch: {rel}")
            continue
        listed.add(path.relative_to(package.resolve()).as_posix().casefold())
        verified += 1

    if verified <= 0:
        errors.append(f"{PACKAGE_HASH_MANIFEST} contains no verified entries")

    manifest_key = manifest_path.resolve().relative_to(package.resolve()).as_posix().casefold()
    actual_files = {
        path.resolve().relative_to(package.resolve()).as_posix().casefold()
        for path in package.rglob("*")
        if path.is_file() and path.resolve() != manifest_path.resolve()
    }
    missing = sorted(actual_files - listed)
    if missing:
        preview = ", ".join(missing[:10])
        suffix = "" if len(missing) <= 10 else f", ... +{len(missing) - 10} more"
        errors.append(f"{PACKAGE_HASH_MANIFEST} does not cover staged files: {preview}{suffix}")
    if manifest_key in listed:
        errors.append(f"{PACKAGE_HASH_MANIFEST} must not hash itself")


def validate_slim_package_layout(package: Path, errors: list[str]) -> None:
    for name in ("LICENSE", "LICENSE_SCOPE.md", "NOTICE", "THIRD_PARTY_NOTICES.md"):
        path = package / name
        if not path.is_file() or path.stat().st_size <= 0:
            errors.append(f"missing staged project license/notice: {name}")

    sharex_dir = package / "ShareX"
    for name in ("KaixinShareX.exe", "LICENSE.txt", "SOURCE_INFO.md"):
        path = sharex_dir / name
        if not path.is_file() or path.stat().st_size <= 0:
            errors.append(f"missing staged ShareX component: ShareX/{name}")
    source_info = sharex_dir / "SOURCE_INFO.md"
    if source_info.is_file():
        text = source_info.read_text(encoding="utf-8", errors="replace")
        if SHAREX_SOURCE_NOTICE_MARKER not in text:
            errors.append(
                "ShareX/SOURCE_INFO.md does not identify the separate corresponding-source archive"
            )
    if (package / "ShareX-Source").exists():
        errors.append("formal staged package still contains ShareX-Source")

    for relative in (
        "srf_ime_translate.exe",
        ".venv-translate",
        "models/translate",
        "tools/kaixin_translate_engine.py",
        "tools/kaixin_translate_engine.cmd",
    ):
        if (package / relative).exists():
            errors.append(f"removed translation runtime is still packaged: {relative}")

    python_roots = [
        package / ".python-runtime",
        package / ".venv-rapidocr",
        package / "RapidOCR-3.9.0" / "python",
    ]
    forbidden: list[str] = []
    for python_root in python_roots:
        if not python_root.is_dir():
            continue
        for path in python_root.rglob("*"):
            name = path.name.casefold()
            if path.is_dir() and (
                name in PYTHON_EXCLUDED_DIR_NAMES
                or (name.startswith("pip-") and name.endswith(".dist-info"))
                or (name.startswith("setuptools-") and name.endswith(".dist-info"))
            ):
                forbidden.append(path.relative_to(package).as_posix())
            elif path.is_file() and path.suffix.casefold() in PYTHON_EXCLUDED_FILE_SUFFIXES:
                forbidden.append(path.relative_to(package).as_posix())
            if len(forbidden) >= 20:
                break
        if len(forbidden) >= 20:
            break
    if forbidden:
        errors.append(
            "Python runtime contains excluded cache/test/development artifacts: "
            + ", ".join(forbidden)
        )


def validate_component_manifest(package: Path, errors: list[str]) -> None:
    package_manifest = package / PACKAGE_HASH_MANIFEST
    component_manifest = package / COMPONENT_MANIFEST
    if not package_manifest.is_file() or not component_manifest.is_file():
        return
    package_lines = [
        line.strip()
        for line in package_manifest.read_text(encoding="utf-8-sig").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    parser = configparser.ConfigParser(interpolation=None)
    try:
        parser.read(component_manifest, encoding="utf-8-sig")
    except Exception as exc:
        errors.append(f"invalid {COMPONENT_MANIFEST}: {exc}")
        return
    for name, prefix in COMPONENT_PREFIXES.items():
        matching = [
            line
            for line in package_lines
            if " *" in line
            and line.split(" *", 1)[1].casefold().startswith(prefix.casefold())
        ]
        expected = (
            hashlib.sha256((("\n".join(matching)) + "\n").encode("utf-8")).hexdigest()
            if matching
            else "absent"
        )
        actual = parser.get("components", name, fallback="").strip().lower()
        if actual != expected:
            errors.append(
                f"{COMPONENT_MANIFEST} component {name!r} mismatch: "
                f"expected={expected} actual={actual or '(missing)'}"
            )


def validate_sharex_source_archive(dist: Path, errors: list[str]) -> None:
    archive_path = dist / SHAREX_SOURCE_ARCHIVE_NAME
    if not archive_path.is_file() or archive_path.stat().st_size <= 0:
        errors.append(f"missing separate ShareX corresponding-source archive: {archive_path.name}")
        return
    required_members = {
        "ShareX-Source/LICENSE.txt",
        "ShareX-Source/SOURCE_INFO.md",
        "ShareX-Source/ShareX/ShareXCLIManager.cs",
        "ShareX-Source/ShareX/Program.cs",
        "ShareX-Source/ShareX/SystemOptions.cs",
    }
    try:
        with zipfile.ZipFile(archive_path) as archive:
            members = {name.replace("\\", "/") for name in archive.namelist()}
    except (OSError, zipfile.BadZipFile) as exc:
        errors.append(f"invalid ShareX corresponding-source archive: {exc}")
        return
    missing = sorted(required_members - members)
    if missing:
        errors.append(
            "ShareX corresponding-source archive is incomplete: " + ", ".join(missing)
        )


def main() -> int:
    repo = Path(__file__).resolve().parents[1]
    dist = repo / "dist"
    package_names = [
        "kaixin-package",
        "kaixin-package-ime",
        "kaixin-package-ocr",
        "kaixin-package-full",
    ]
    packages = [dist / name for name in package_names if (dist / name).exists()]
    if not packages:
        print("Package manifest check skipped: no staged package directories found")
        return 0

    errors: list[str] = []
    validate_sharex_source_archive(dist, errors)
    for package in packages:
        package_label = package.relative_to(repo).as_posix()
        for name in REQUIRED_FILES:
            if not (package / name).is_file():
                errors.append(f"{package_label}: missing staged package file: {name}")

        before = len(errors)
        validate_package_hash_manifest(package, errors)
        validate_slim_package_layout(package, errors)
        validate_component_manifest(package, errors)
        for index in range(before, len(errors)):
            errors[index] = f"{package_label}: {errors[index]}"

        manifest_path = package / "build-manifest.json"
        if manifest_path.is_file():
            try:
                manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
            except Exception as exc:
                errors.append(f"{package_label}: invalid build-manifest.json: {exc}")
            else:
                for key in ["profile", "artifacts", "runtime_payload"]:
                    if key not in manifest:
                        errors.append(f"{package_label}: build-manifest.json missing key: {key}")

        payload_manifest = package / "current_runtime_payload.txt"
        if payload_manifest.is_file():
            rel = payload_manifest.read_text(encoding="ascii", errors="strict").strip()
            payload = (package / rel).resolve()
            try:
                payload.relative_to(package.resolve())
            except ValueError:
                errors.append(f"{package_label}: runtime payload escapes staged package root")
            if not (payload / "x64" / "srf_tsf_tip.dll").is_file():
                errors.append(f"{package_label}: runtime payload missing x64/srf_tsf_tip.dll")
            if not (payload / "x86" / "srf_tsf_tip.dll").is_file():
                errors.append(f"{package_label}: runtime payload missing x86/srf_tsf_tip.dll")

    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print("Package manifest check passed: " + ", ".join(p.name for p in packages))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
