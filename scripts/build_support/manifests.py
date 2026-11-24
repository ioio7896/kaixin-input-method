from __future__ import annotations
import hashlib
import re
import configparser
from pathlib import Path, PurePosixPath
from datetime import datetime, timezone
from .config import (
    PACKAGE_HASH_MANIFEST,
    COMPONENT_MANIFEST,
    COMPONENT_PREFIXES,
    PE_MACHINE_BY_ARCH,
    RAPIDOCR_DIR,
    RAPIDOCR_DLL_NAME,
    RAPIDOCR_REQUIRED_MODELS,
)


def validate_rapidocr_layout(package: Path, include_ocr: bool) -> list[str]:
    errors: list[str] = []
    root = package / RAPIDOCR_DIR
    if not include_ocr:
        if root.exists() or root.is_symlink():
            errors.append(f"IME-only package must not include {RAPIDOCR_DIR}")
        return errors
    if not root.is_dir() or root.is_symlink():
        return [f"missing or invalid staged OCR directory: {RAPIDOCR_DIR}"]

    models = "models"
    directories = {models}
    required = {
        "LICENSE",
        "README.md",
        "README-CN.md",
        RAPIDOCR_DLL_NAME,
        *(f"{models}/{name}" for name in RAPIDOCR_REQUIRED_MODELS),
    }
    allowed = required | {"windows-utf8.patch", f"{models}/PP-OCRv6_det_int8.onnx"}
    seen = set()
    for path in sorted(root.rglob("*")):
        relative = path.relative_to(root).as_posix()
        if not path.is_symlink() and path.is_dir() and relative in directories:
            continue
        if not path.is_symlink() and path.is_file() and relative in allowed:
            seen.add(relative)
            if path.stat().st_size == 0:
                errors.append(f"empty staged OCR file: {RAPIDOCR_DIR}/{relative}")
        else:
            errors.append(f"unexpected staged OCR entry: {RAPIDOCR_DIR}/{relative}")
    errors.extend(
        f"missing staged OCR file: {RAPIDOCR_DIR}/{name}" for name in sorted(required - seen)
    )
    return errors


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
            if " *" in line and line.split(" *", 1)[1].casefold().startswith(prefix.casefold())
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
