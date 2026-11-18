"""Verified release: one Shanghai timestamp and exactly two installers."""

from __future__ import annotations

import argparse
from datetime import datetime
import hashlib
import json
from pathlib import Path
import subprocess
import sys

from release_version import SHANGHAI, timestamp_version, update_version, validate_sources
from build_support.manifests import verify_package_hash_manifest

ROOT = Path(__file__).resolve().parents[1]


def validate_pair(repo: Path, version: str, before: set[Path]) -> list[Path]:
    installers = []
    for variant in ("ime", "ocr"):
        matches = [
            p
            for p in (repo / "dist").glob(f"kaixin-setup-{variant}-{version}-*.exe")
            if p not in before
        ]
        if len(matches) != 1 or matches[0].stat().st_size == 0:
            raise RuntimeError(
                f"Release requires one new nonempty {variant} installer; found {len(matches)}"
            )
        stage = repo / "dist" / f"kaixin-package-{variant}"
        if (stage / "VERSION").read_text(encoding="utf-8").strip() != version:
            raise RuntimeError(f"Staged {variant} version differs from release {version}")
        verify_package_hash_manifest(stage)
        installers.extend(matches)
    timestamps = {path.name.split(f"-{version}-", 1)[1] for path in installers}
    if len(timestamps) != 1:
        raise RuntimeError("The two installers must come from the same build invocation")
    return installers


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sign", action="store_true")
    parser.add_argument("--sign-required", action="store_true")
    parser.add_argument(
        "--skip-tests",
        action="store_true",
        help="explicitly skip code tests and smoke checks; record the omission in the release report",
    )
    args = parser.parse_args()
    version = timestamp_version(datetime.now(SHANGHAI))
    update_version(ROOT, version)
    errors = validate_sources(ROOT)
    if errors:
        raise RuntimeError("\n".join(errors))
    before = set((ROOT / "dist").glob("kaixin-setup-*.exe"))
    command = [
        sys.executable,
        str(ROOT / "build.py"),
        "--package-variants",
        "all",
    ]
    command.extend(
        ["--no-verify", "--no-smoke", "--skip-runtime-checks"]
        if args.skip_tests
        else ["--verify", "--smoke"]
    )
    if args.sign or args.sign_required:
        command.append("--sign-required" if args.sign_required else "--sign")
    subprocess.run(command, cwd=ROOT, check=True)
    installers = validate_pair(ROOT, version, before)
    records = []
    for path in installers:
        with path.open("rb") as handle:
            digest = hashlib.file_digest(handle, "sha256").hexdigest()
        records.append({"name": path.name, "bytes": path.stat().st_size, "sha256": digest})
    report = {
        "version": version,
        "verification": "skipped_by_request" if args.skip_tests else "passed",
        "installers": records,
    }
    manifest = ROOT / "dist" / f"release-{version}.json"
    manifest.write_text(json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"Verified release: {manifest}")


if __name__ == "__main__":
    main()
