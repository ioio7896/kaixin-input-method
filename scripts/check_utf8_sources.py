#!/usr/bin/env python3
from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from typing import Any, cast


EXCLUDED_DIR_NAMES = {
    ".cache",
    ".git",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".venv",
    ".venv-rapidocr",
    "__pycache__",
    "build",
    "build-codex",
    "build-package",
    "build-package-x86",
    "dist",
    "models",
    "node_modules",
    "site-packages",
    "target",
    "venv",
    "外挂词库",
}

EXCLUDED_FILE_NAMES = {
    # Local Windows clipboard/text export scratch files can be UTF-16LE and are not source inputs.
    "\u526a\u8d34\u677f\u6587\u672c.txt",
}

MOJIBAKE_CHECK_EXCLUDED_DIR_NAMES = {
    "data",
    "lexicon",
}

TEXT_SUFFIXES = {
    ".cmake",
    ".cmd",
    ".cpp",
    ".c",
    ".def",
    ".filters",
    ".h",
    ".hpp",
    ".ini",
    ".ipp",
    ".iss",
    ".json",
    ".lock",
    ".md",
    ".manifest",
    ".py",
    ".rc",
    ".rs",
    ".slnx",
    ".ps1",
    ".toml",
    ".txt",
    ".yaml",
    ".yml",
}

TEXT_FILENAMES = {
    ".clang-format",
    ".editorconfig",
    ".gitignore",
    "license",
    "notice",
    "readme",
    "version",
}

MOJIBAKE_FRAGMENTS = (
    "ï¿½",
    "�",
    "寮€",
    "鈥",
    "銆",
    "濄€",
    "鐨",
    "鐢",
    "鍙",
    "鍒",
    "绋",
    "鏂",
    "浠",
    "叆娉",
)

MOJIBAKE_PREFIXES = (
    "缂",
    "鍓",
    "鐑",
    "棰",
    "撳",
    "锛",
    "涓",
    "妯",
    "鏌",
    "绠",
    "娣",
    "鍏",
)

MOJIBAKE_RE = re.compile(
    "|".join(
        [re.escape(fragment) for fragment in MOJIBAKE_FRAGMENTS]
        + [rf"{re.escape(prefix)}[^\sA-Za-z0-9]{{1,8}}" for prefix in MOJIBAKE_PREFIXES]
    )
)


def resolve_repo_root() -> Path:
    return Path(__file__).resolve().parents[1]


def is_excluded(path: Path) -> bool:
    return any(
        part in EXCLUDED_DIR_NAMES
        or part.startswith(".venv")
        or part.startswith("build-")
        or part.startswith("kaixin-diagnostics-")
        for part in path.parts
    )


def is_text_candidate(path: Path) -> bool:
    name = path.name.lower()
    if name in TEXT_FILENAMES:
        return True
    if path.suffix.lower() in TEXT_SUFFIXES:
        return True
    if name.endswith(".vcxproj"):
        return True
    return False


def should_check_mojibake(path: Path) -> bool:
    if path.name == "check_utf8_sources.py":
        return False
    if any(part in MOJIBAKE_CHECK_EXCLUDED_DIR_NAMES for part in path.parts):
        return False
    return path.suffix.lower() not in {".json", ".lock", ".txt"}


def iter_text_files(root: Path):
    def walk_dir(directory: Path):
        try:
            entries = sorted(directory.iterdir(), key=lambda p: p.name.lower())
        except OSError:
            return
        for path in entries:
            rel = path.relative_to(root)
            if path.is_dir():
                if not is_excluded(rel):
                    yield from walk_dir(path)
                continue
            if not path.is_file():
                continue
            if is_excluded(rel):
                continue
            if rel.name.lower() in EXCLUDED_FILE_NAMES:
                continue
            if not is_text_candidate(path):
                continue
            yield path

    yield from walk_dir(root)


def check_file(root: Path, path: Path) -> str | None:
    rel = path.relative_to(root).as_posix()
    try:
        text = path.read_text(encoding="utf-8")
    except UnicodeDecodeError as exc:
        return f"{rel}: invalid UTF-8 at byte {exc.start} ({exc.reason})"
    except OSError as exc:
        return f"{rel}: {exc}"
    if should_check_mojibake(path.relative_to(root)):
        for line_no, line in enumerate(text.splitlines(), 1):
            if MOJIBAKE_RE.search(line):
                return f"{rel}:{line_no}: likely mojibake text: {line.strip()[:120]}"
    return None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Check that repository text sources are UTF-8 encoded.",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="repository root to scan (default: repo root inferred from this script)",
    )
    return parser.parse_args()


def main() -> int:
    for stream in (sys.stdout, sys.stderr):
        try:
            reconfigure = getattr(cast(Any, stream), "reconfigure", None)
            if reconfigure is not None:
                reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, OSError, TypeError, ValueError):
            pass

    args = parse_args()
    root = (args.root or resolve_repo_root()).resolve()
    if not root.is_dir():
        print(f"root directory not found: {root}")
        return 2

    files = list(iter_text_files(root))
    if not files:
        print(f"UTF-8 check passed: 0 text files scanned under {root}")
        return 0

    issues: list[str] = []
    for path in files:
        issue = check_file(root, path)
        if issue:
            issues.append(issue)

    if issues:
        print(f"UTF-8 check failed: {len(issues)} file(s) are not valid UTF-8")
        for issue in issues:
            print(f"  - {issue}")
        return 1

    print(f"UTF-8 check passed: {len(files)} text file(s) scanned")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
