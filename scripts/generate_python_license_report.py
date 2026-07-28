#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Generate a dependency/license report using only Python's standard library."""

from __future__ import annotations

import argparse
import importlib.metadata as metadata
from pathlib import Path


LICENSE_NAMES = ("license", "copying", "notice", "copyright", "authors")


def is_license_file(path: Path) -> bool:
    name = path.name.lower()
    return any(name.startswith(prefix) for prefix in LICENSE_NAMES) or any(
        part.lower() in {"license", "licenses"} for part in path.parts
    )


def clean(value: str | None) -> str:
    return " ".join((value or "").split())


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("output", type=Path)
    args = parser.parse_args()

    distributions = sorted(
        metadata.distributions(),
        key=lambda dist: clean(dist.metadata.get("Name")).casefold(),
    )
    lines = [
        "# Python dependency licenses",
        "",
        "Generated from the exact Python environment used to run this script.",
        "",
        "| Package | Version | Declared license | Project URL |",
        "| --- | --- | --- | --- |",
    ]
    details: list[str] = []
    for dist in distributions:
        package = clean(dist.metadata.get("Name")) or "unknown"
        version = clean(dist.version)
        expression = clean(dist.metadata.get("License-Expression"))
        declared = expression or clean(dist.metadata.get("License"))
        if not declared:
            classifiers = dist.metadata.get_all("Classifier", [])
            declared = "; ".join(
                item.removeprefix("License ::").strip()
                for item in classifiers
                if item.startswith("License ::")
            )
        declared = declared or "NOT DECLARED"
        project_url = clean(dist.metadata.get("Home-page"))
        if not project_url:
            for entry in dist.metadata.get_all("Project-URL", []):
                if "," in entry:
                    _, project_url = entry.split(",", 1)
                    project_url = project_url.strip()
                    break
        package_cell = package.replace("|", "\\|")
        declared_cell = declared.replace("|", "\\|")
        project_url_cell = project_url.replace("|", "\\|")
        lines.append(
            f"| {package_cell} | {version} | {declared_cell} | {project_url_cell} |"
        )

        license_files = []
        for item in dist.files or []:
            relative = Path(str(item))
            if not is_license_file(relative):
                continue
            absolute = Path(dist.locate_file(item))
            if absolute.is_file() and absolute.stat().st_size <= 1024 * 1024:
                license_files.append((relative, absolute))
        details.extend(["", f"## {package} {version}", "", f"Declared: `{declared}`", ""])
        if not license_files:
            details.append("No bundled license/notice text was found in the installed distribution.")
        for relative, absolute in license_files:
            try:
                content = absolute.read_text(encoding="utf-8", errors="replace").strip()
            except OSError as exc:
                content = f"Unable to read license file: {exc}"
            details.extend([f"### `{relative.as_posix()}`", "", "```text", content, "```", ""])

    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("\n".join(lines + details) + "\n", encoding="utf-8")
    print(f"wrote {len(distributions)} Python dependencies to {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
