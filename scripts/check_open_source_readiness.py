#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Check that a prospective public repository excludes private/build data."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
REQUIRED = (
    "LICENSE",
    "LICENSE_SCOPE.md",
    "NOTICE",
    "THIRD_PARTY_NOTICES.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "CODE_OF_CONDUCT.md",
    "CHANGELOG.md",
    ".github/pull_request_template.md",
    ".github/ISSUE_TEMPLATE/bug_report.yml",
    "docs/licenses/OCR_MODELS.md",
    "docs/licenses/S2T_SOURCE.md",
    "docs/licenses/CORPUS_SOURCE.md",
    "docs/licenses/PROJECT_DATA.md",
    "docs/licenses/AI_DATA_PROVENANCE.md",
    "docs/licenses/AI_DATA_VERIFICATION.md",
)
FORBIDDEN_PARTS = {
    ".venv-rapidocr",
    "__pycache__",
    "build",
    "dist",
    "bin",
    "obj",
    "publish",
}
FORBIDDEN_SUFFIXES = {".pfx", ".p12", ".pem", ".key", ".dmp", ".pdb"}
FORBIDDEN_NAMES = {
    "pinyin-ime/data/corpus.txt",
}
HIGH_CONFIDENCE_SECRETS = (
    re.compile(rb"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    re.compile(rb"AKIA[0-9A-Z]{16}"),
    re.compile(rb"gh[pousr]_[A-Za-z0-9_]{20,}"),
    re.compile(rb"sk-[A-Za-z0-9_-]{20,}"),
)


def git_tracked_files() -> list[Path] | None:
    probe = subprocess.run(
        ["git", "rev-parse", "--is-inside-work-tree"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if probe.returncode != 0:
        return None
    # Include untracked, non-ignored files so the check is useful before the
    # first commit as well as in CI after files have been staged.
    result = subprocess.run(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT,
        capture_output=True,
        check=True,
    )
    return [Path(item.decode("utf-8")) for item in result.stdout.split(b"\0") if item]


def status_value(path: Path) -> str | None:
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.lower().startswith("status:"):
            return line.split(":", 1)[1].strip().lower()
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--release",
        action="store_true",
        help="also fail when a third-party provenance record remains blocked",
    )
    args = parser.parse_args()

    errors: list[str] = []
    warnings: list[str] = []
    if args.release:
        verification = subprocess.run(
            [sys.executable, str(ROOT / "scripts/verify_ai_generated_data.py")],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        if verification.returncode != 0:
            errors.append(
                "AI-assisted data verification failed:\n"
                + (verification.stderr or verification.stdout).strip()
            )
    for relative in REQUIRED:
        if not (ROOT / relative).is_file():
            errors.append(f"missing required public-repository file: {relative}")

    cargo_toml = (ROOT / "pinyin-ime/Cargo.toml").read_text(encoding="utf-8")
    for field in ('license = "Apache-2.0"', "rust-version =", "readme ="):
        if field not in cargo_toml:
            errors.append(f"Cargo.toml is missing package metadata: {field}")

    license_records = sorted((ROOT / "docs/licenses").glob("*.md"))
    if args.release:
        for record in license_records:
            status = status_value(record)
            if status == "blocked":
                errors.append(f"release-blocking license record: {record.relative_to(ROOT)}")
            elif record.name != "README.md" and status != "verified":
                errors.append(f"license record has no verified status: {record.relative_to(ROOT)}")

    tracked = git_tracked_files()
    if tracked is None:
        warnings.append("not an initialized Git worktree; tracked-file checks were skipped")
    else:
        for relative in tracked:
            posix = relative.as_posix()
            lower_parts = {part.lower() for part in relative.parts}
            if posix in FORBIDDEN_NAMES:
                errors.append(f"unverified generated data is tracked: {posix}")
            if lower_parts & FORBIDDEN_PARTS:
                errors.append(f"build/private directory is tracked: {posix}")
            if relative.suffix.lower() in FORBIDDEN_SUFFIXES:
                errors.append(f"private or generated file is tracked: {posix}")
            if relative.parts and relative.parts[0].startswith("kaixin-diagnostics-"):
                errors.append(f"diagnostic export is tracked: {posix}")
            absolute = ROOT / relative
            if absolute.is_file() and absolute.stat().st_size > 20 * 1024 * 1024:
                errors.append(f"tracked file exceeds 20 MiB: {posix}")
            if absolute.is_file() and absolute.stat().st_size <= 5 * 1024 * 1024:
                data = absolute.read_bytes()
                if any(pattern.search(data) for pattern in HIGH_CONFIDENCE_SECRETS):
                    errors.append(f"possible high-confidence secret in tracked file: {posix}")

    for warning in warnings:
        print(f"warning: {warning}")
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    if errors:
        print(f"open-source readiness failed with {len(errors)} error(s)", file=sys.stderr)
        return 1
    print("open-source repository structure checks passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
